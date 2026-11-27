//! `jade-debug launch`: start the app with the debug socket on, ready to drive.

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use jade_debug_protocol::{Answer, Command, Query};

use crate::args::LaunchArgs;
use crate::client::Connection;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_COPY_DEPTH: usize = 32;

pub(crate) fn run(socket: &Path, args: LaunchArgs) -> Result<(), String> {
    let bin = find_app(args.bin)?;
    let vault = args
        .vault
        .canonicalize()
        .map_err(|e| format!("{}: {e}", args.vault.display()))?;
    let vault = if args.in_place {
        eprintln!(
            "warning: --in-place, so typing in the app autosaves into {}",
            vault.display()
        );
        vault
    } else {
        copy_vault(&vault)?
    };
    let _ = std::fs::remove_file(socket); // a stale socket from a previous run
    let mut app = std::process::Command::new(&bin);
    app.arg(&vault);
    if args.headless {
        app.env("JADE_DEBUG_HEADLESS", "1");
        if let Some(size) = &args.size {
            app.env("JADE_DEBUG_SIZE", size);
        }
    }
    // Survives this CLI exiting or being interrupted.
    app.process_group(0);
    let (child, log) = spawn(app, socket)?;
    println!(
        "pid={} socket={} vault={} log={}",
        child.id(),
        socket.display(),
        vault.display(),
        log.display()
    );
    println!("app: {}", bin.display());
    Ok(())
}

/// Starts the app `command` with its socket at `socket` and its output in a
/// log file beside it, and waits until it answers. Returns the process and the
/// log's path.
pub(crate) fn spawn(
    mut command: std::process::Command,
    socket: &Path,
) -> Result<(Child, PathBuf), String> {
    let log = PathBuf::from(format!("{}.log", socket.display()));
    let log_file = File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let program = command.get_program().to_owned();
    let mut child = command
        .env("JADE_DEBUG_SOCKET", socket)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    if let Err(err) = wait_until_ready(socket, &mut child, &log) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }
    Ok((child, log))
}

pub(crate) fn find_app(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("jade-app")));
    let candidates = std::env::var_os("JADE_APP_BIN")
        .map(PathBuf::from)
        .into_iter()
        .chain(sibling)
        .chain([
            PathBuf::from("target/release/jade-app"),
            PathBuf::from("target/debug/jade-app"),
        ]);
    first_existing(candidates)
}

fn first_existing(candidates: impl IntoIterator<Item = PathBuf>) -> Result<PathBuf, String> {
    let mut tried = Vec::new();
    for path in candidates {
        if path.is_file() {
            return Ok(path);
        }
        tried.push(path.display().to_string());
    }
    Err(format!(
        "jade-app not found (tried {}); build it or pass --bin PATH",
        tried.join(", ")
    ))
}

fn copy_vault(src: &Path) -> Result<PathBuf, String> {
    let name = src
        .file_name()
        .map_or_else(|| "vault".into(), |n| n.to_string_lossy().into_owned());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dst = std::env::temp_dir().join(format!("jade-debug-vault-{stamp}-{name}"));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(src, &dst, 0).map_err(|e| format!("copying the vault to {}: {e}", dst.display()))?;
    eprintln!("using a copy of the vault: {}", dst.display());
    Ok(dst)
}

/// Copies regular files and directories; symlinks and `.git` are skipped.
fn copy_dir(src: &Path, dst: &Path, depth: usize) -> std::io::Result<()> {
    if depth > MAX_COPY_DEPTH {
        return Err(std::io::Error::other("vault nests too deeply"));
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if kind.is_dir() && entry.file_name() != ".git" {
            copy_dir(&entry.path(), &target, depth + 1)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn wait_until_ready(socket: &Path, child: &mut Child, log: &Path) -> Result<(), String> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let tail = std::fs::read_to_string(log).unwrap_or_default();
            return Err(format!(
                "the app exited early ({status}); log {}:\n{tail}",
                log.display()
            ));
        }
        if let Ok(mut conn) = Connection::open(socket)
            && let Ok((Answer::WindowSize { .. }, _)) = conn.call(Command::Query {
                what: Query::WindowSize,
            })
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "the app did not become ready within {READY_TIMEOUT:?}; log {} (was jade-app built \
         with `--features debug-socket`? Without it there is no socket)",
        log.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jade-cli-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copying_a_vault_keeps_notes_and_skips_git_and_symlinks() {
        let src = scratch("src");
        std::fs::create_dir_all(src.join("folder/deeper")).unwrap();
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join("a.md"), "A").unwrap();
        std::fs::write(src.join("folder/deeper/b.md"), "B").unwrap();
        std::fs::write(src.join(".git/config"), "x").unwrap();
        std::os::unix::fs::symlink(src.join("a.md"), src.join("link.md")).unwrap();

        let dst = scratch("dst");
        copy_dir(&src, &dst, 0).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("a.md")).unwrap(), "A");
        assert_eq!(
            std::fs::read_to_string(dst.join("folder/deeper/b.md")).unwrap(),
            "B"
        );
        assert!(!dst.join(".git").exists());
        assert!(!dst.join("link.md").exists());
        std::fs::remove_dir_all(src).unwrap();
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn an_explicit_binary_wins_and_a_missing_one_lists_what_was_tried() {
        assert_eq!(
            find_app(Some("/x/app".into())).unwrap(),
            PathBuf::from("/x/app")
        );
        let dir = scratch("bin");
        let real = dir.join("jade-app");
        std::fs::write(&real, "").unwrap();
        let missing = dir.join("nope");
        assert_eq!(
            first_existing([missing.clone(), real.clone()]).unwrap(),
            real
        );
        let message = first_existing([missing.clone()]).unwrap_err();
        assert!(
            message.contains(&*missing.to_string_lossy()) && message.contains("--bin"),
            "{message}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
