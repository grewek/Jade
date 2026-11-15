//! Opt-in process-memory logging: run with `JADE_MEMORY_LOG=1` and RSS is
//! printed to stderr every two seconds, so a memory report comes with
//! numbers instead of a `btop` screenshot.

use std::time::Duration;

const ENV_VAR: &str = "JADE_MEMORY_LOG";
const INTERVAL: Duration = Duration::from_secs(2);

/// Resident set size in KiB, from `/proc/self/status` (Linux only).
pub(crate) fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vm_rss_kib(&status)
}

fn parse_vm_rss_kib(status: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Starts the logging thread if `JADE_MEMORY_LOG` is set; otherwise a no-op.
pub(crate) fn start_if_enabled() {
    if std::env::var_os(ENV_VAR).is_none() {
        return;
    }
    std::thread::spawn(|| {
        loop {
            match rss_kib() {
                Some(kib) => eprintln!("[jade-memory] rss={} MiB", kib / 1024),
                None => eprintln!("[jade-memory] rss unavailable on this platform"),
            }
            std::thread::sleep(INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_vm_rss_kib;

    #[test]
    fn parses_the_vmrss_line() {
        let status = "Name:\tjade\nVmRSS:\t  123456 kB\nThreads:\t4\n";
        assert_eq!(parse_vm_rss_kib(status), Some(123456));
    }

    #[test]
    fn missing_vmrss_is_none() {
        assert_eq!(parse_vm_rss_kib("Name:\tjade\n"), None);
    }
}
