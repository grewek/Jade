//! Opt-in debug channel: with `JADE_DEBUG_SOCKET=<path>` the app listens on a
//! Unix socket and broadcasts newline-delimited text lines (the memory probe and
//! the editor's paint trace) to every connected client, so a tool can read the live app without
//! scraping a terminal. Read-only: nothing a client sends is interpreted.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Mutex;
use std::time::Duration;

const ENV_VAR: &str = "JADE_DEBUG_SOCKET";
/// A stuck client must not stall the emitting thread.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

static CLIENTS: Mutex<Vec<UnixStream>> = Mutex::new(Vec::new());

/// Whether the socket was requested; lets producers skip work otherwise.
pub(crate) fn requested() -> bool {
    std::env::var_os(ENV_VAR).is_some()
}

/// Binds the socket and starts accepting clients. No-op unless requested;
/// failures are reported on stderr and leave the app running.
pub(crate) fn start_if_enabled() {
    let Some(path) = std::env::var_os(ENV_VAR) else {
        return;
    };
    let _ = std::fs::remove_file(&path); // stale socket from a previous run
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => return eprintln!("[jade-debug] cannot bind {path:?}: {err}"),
    };
    if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        return eprintln!("[jade-debug] cannot restrict {path:?}: {err}");
    }
    eprintln!("[jade-debug] listening on {path:?}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
                continue;
            }
            let mut stream = stream;
            if stream.write_all(b"[jade-debug] connected\n").is_ok()
                && let Ok(mut clients) = CLIENTS.lock()
            {
                clients.push(stream);
            }
        }
    });
}

/// Sends `line` to every client, dropping any that fail.
pub(crate) fn emit(line: &str) {
    let Ok(mut clients) = CLIENTS.lock() else {
        return;
    };
    if clients.is_empty() {
        return;
    }
    let msg = format!("{line}\n");
    clients.retain_mut(|c| match c.write_all(msg.as_bytes()) {
        Ok(()) => true,
        Err(err) => {
            eprintln!(
                "[jade-debug] dropped a client ({} byte line): {err}",
                msg.len()
            );
            false
        }
    });
}

/// Blocks (up to ~1s) until a client has been registered, for tests.
#[cfg(test)]
pub(crate) fn wait_for_client() {
    for _ in 0..100 {
        if CLIENTS.lock().is_ok_and(|c| !c.is_empty()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn a_connected_client_receives_emitted_lines() {
        let path =
            std::env::temp_dir().join(format!("jade-debug-test-{}.sock", std::process::id()));
        // SAFETY: test runs single-threaded (`--test-threads=1`).
        unsafe { std::env::set_var(ENV_VAR, &path) };
        start_if_enabled();
        let client = UnixStream::connect(&path).unwrap();
        let mut reader = BufReader::new(client);
        let mut hello = String::new();
        reader.read_line(&mut hello).unwrap();
        assert!(hello.contains("connected"));
        // The accept thread registers the client right after the hello.
        for _ in 0..100 {
            emit("ping");
            if CLIENTS.lock().unwrap().len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut got = String::new();
        reader.read_line(&mut got).unwrap();
        assert_eq!(got, "ping\n");
        std::fs::remove_file(&path).unwrap();
        unsafe { std::env::remove_var(ENV_VAR) };
    }
}
