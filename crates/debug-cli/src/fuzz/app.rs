//! The app under test: a headless process on a throwaway vault and config
//! folder, and its socket.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use jade_debug_protocol::{Answer, Command};

use crate::client::Connection;
use crate::launch;

/// The note every run edits.
pub(super) const NOTE: &str = "Note";
const START_SIZE: &str = "1000x700";
/// What the app answers when its main thread is busy or dead.
const NO_REPLY: &str = "no reply from the UI thread";
/// How long a process that stopped answering gets to finish exiting.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Why the app did not answer.
#[derive(Debug)]
pub(super) enum Gone {
    Crash(String),
    Hang(String),
}

pub(super) struct App {
    child: Child,
    conn: Connection,
    dir: PathBuf,
    log: PathBuf,
}

impl App {
    /// Starts the app on a vault holding `note` alone, with default settings.
    pub(super) fn start(bin: &Path, note: &str, seed: u64) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("jade-fuzz-{}-{seed}", std::process::id()));
        let (vault, config) = (dir.join("vault"), dir.join("config"));
        let io = |err: std::io::Error| format!("{}: {err}", dir.display());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&vault).map_err(io)?;
        std::fs::create_dir_all(&config).map_err(io)?;
        std::fs::write(vault.join(format!("{NOTE}.md")), note).map_err(io)?;
        let socket = dir.join("sock");
        let mut command = std::process::Command::new(bin);
        command
            .arg(&vault)
            .env("JADE_DEBUG_HEADLESS", "1")
            .env("JADE_DEBUG_SIZE", START_SIZE)
            // Never the user's own settings: they would change the results.
            .env("JADE_CONFIG_DIR", &config);
        let (child, log) = launch::spawn(command, &socket)?;
        let conn = Connection::open(&socket)?;
        Ok(Self {
            child,
            conn,
            dir,
            log,
        })
    }

    /// The answer to `command`. An error the app reports is an answer.
    pub(super) fn call(&mut self, command: Command) -> Result<Answer, Gone> {
        match self.conn.call(command) {
            Ok((Answer::Error { message }, _)) if message.starts_with(NO_REPLY) => {
                Err(self.why_gone(&message))
            }
            Ok((answer, _)) => Ok(answer),
            Err(err) => Err(self.why_gone(&err)),
        }
    }

    fn why_gone(&mut self, symptom: &str) -> Gone {
        let deadline = Instant::now() + EXIT_WAIT;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    let log = std::fs::read_to_string(&self.log).unwrap_or_default();
                    return Gone::Crash(
                        panic_message(&log).unwrap_or_else(|| format!("the app ended: {status}")),
                    );
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => return Gone::Hang(symptom.to_owned()),
            }
        }
    }

    /// Quits the app; a panic on the way out is still a crash.
    pub(super) fn quit(mut self) -> Result<(), Gone> {
        self.call(Command::Quit)?;
        let deadline = Instant::now() + EXIT_WAIT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => return Err(self.why_gone("quit")),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        Err(Gone::Hang("the app did not exit after quit".into()))
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Already gone after `quit`; both calls then fail harmlessly.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Where the first panic in `log` happened, and its message.
fn panic_message(log: &str) -> Option<String> {
    let mut lines = log.lines().skip_while(|l| !l.contains(" panicked at "));
    let first = lines.next()?;
    let location = &first[first.find("panicked at ")?..];
    let message: Vec<&str> = lines
        .take_while(|l| !l.starts_with("note: ") && !l.starts_with("stack backtrace"))
        .take(8)
        .collect();
    Some(format!("{location}\n{}", message.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_panic_is_taken_from_the_log() {
        let log = "[jade-debug] ready\nthread 'main' (7) panicked at src/a.rs:3:5:\nbad index\n\
                   note: run with `RUST_BACKTRACE=1`\nthread 'x' (8) panicked at b.rs:1:1:\nlater\n";
        assert_eq!(
            panic_message(log).as_deref(),
            Some("panicked at src/a.rs:3:5:\nbad index")
        );
        assert_eq!(panic_message("all fine\n"), None);
    }
}
