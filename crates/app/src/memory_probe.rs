//! Opt-in process-memory logging: run with `JADE_MEMORY_LOG=1` and RSS is
//! printed to stderr every two seconds, so a memory report comes with
//! numbers instead of a `btop` screenshot.

use std::time::Duration;

use jade_debug_protocol::{Event, MemoryInfo, Topic};

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

/// glibc's `mallinfo2` result (field order is ABI; all sizes in bytes).
#[repr(C)]
struct MallInfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn mallinfo2() -> MallInfo2;
}

/// `(in_use, free_but_retained, mmapped)` MiB from glibc's allocator. Tells
/// fragmentation (large `free_but_retained`) from live growth (`in_use`).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn allocator_mib() -> Option<(usize, usize, usize)> {
    // SAFETY: `mallinfo2` takes no arguments and only reads allocator state.
    let info = unsafe { mallinfo2() };
    const MIB: usize = 1024 * 1024;
    Some((info.uordblks / MIB, info.fordblks / MIB, info.hblkhd / MIB))
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn allocator_mib() -> Option<(usize, usize, usize)> {
    None
}

/// One sample of process memory; `None` where RSS is unavailable.
pub(crate) fn sample() -> Option<MemoryInfo> {
    let heap = allocator_mib();
    Some(MemoryInfo {
        rss_kib: rss_kib()?,
        heap_in_use_mib: heap.map(|h| h.0 as u64),
        heap_free_retained_mib: heap.map(|h| h.1 as u64),
        mmapped_mib: heap.map(|h| h.2 as u64),
    })
}

fn log_line(info: &MemoryInfo) -> String {
    let rss = info.rss_kib / 1024;
    match (
        info.heap_in_use_mib,
        info.heap_free_retained_mib,
        info.mmapped_mib,
    ) {
        (Some(used), Some(free), Some(mapped)) => format!(
            "[jade-memory] rss={rss} MiB heap_in_use={used} MiB \
             heap_free_retained={free} MiB mmapped={mapped} MiB"
        ),
        _ => format!("[jade-memory] rss={rss} MiB"),
    }
}

/// Starts the sampling thread if `JADE_MEMORY_LOG` or `JADE_DEBUG_SOCKET` is
/// set: samples go to stderr (with the log variable) and to clients subscribed
/// to the memory topic. Samples are only taken when someone consumes them.
pub(crate) fn start_if_enabled() {
    if std::env::var_os(ENV_VAR).is_none() && !crate::debug_server::requested() {
        return;
    }
    std::thread::spawn(|| {
        loop {
            let log = std::env::var_os(ENV_VAR).is_some();
            if (log || crate::debug_server::wants(Topic::Memory))
                && let Some(info) = sample()
            {
                if log {
                    eprintln!("{}", log_line(&info));
                }
                crate::debug_server::emit(&Event::Memory(info));
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

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn allocator_stats_reflect_a_live_allocation() {
        let keep = std::hint::black_box(vec![1u8; 64 * 1024 * 1024]);
        let (used, _, mapped) = super::allocator_mib().unwrap();
        assert!(used + mapped >= 64, "used={used} mapped={mapped}");
        std::hint::black_box(&keep);
    }

    #[test]
    fn missing_vmrss_is_none() {
        assert_eq!(parse_vm_rss_kib("Name:\tjade\n"), None);
    }
}
