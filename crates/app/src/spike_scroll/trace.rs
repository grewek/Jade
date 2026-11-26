//! One line per frame and per wheel event. Diagnostics only.

use std::io::Write;
use std::time::{Duration, Instant};

use std::sync::Mutex;

static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);

/// Time since the previous call, in µs.
pub(crate) fn interval_us(now: Instant) -> u128 {
    let Ok(mut last) = LAST_FRAME.lock() else {
        return 0;
    };
    let interval = last.map_or(Duration::ZERO, |last| now.duration_since(last));
    *last = Some(now);
    interval.as_micros()
}

/// Appends to `JADE_SPIKE_LOG`, if set.
pub(crate) fn log(line: &str) {
    let Some(path) = std::env::var_os("JADE_SPIKE_LOG") else {
        return;
    };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(err) = written {
        eprintln!("[spike] cannot write the log: {err}");
    }
}
