//! Per-frame timing of the editor canvas for the `frames` topic and
//! `query frame_times`. Recording is gated on the debug server being on, and
//! costs two clock reads plus one uncontended lock per frame into a fixed ring
//! (no allocation).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use jade_debug_protocol::{Event, FRAME_HISTORY, FrameSample, Topic};

static FRAME: AtomicU64 = AtomicU64::new(0);
static HISTORY: Mutex<History> = Mutex::new(History::new());

const EMPTY: FrameSample = FrameSample {
    frame: 0,
    render_us: 0,
    paint_us: 0,
    interval_us: 0,
};

struct History {
    samples: [FrameSample; FRAME_HISTORY],
    /// Slot the next sample goes into.
    next: usize,
    len: usize,
    last_start: Option<Instant>,
}

impl History {
    const fn new() -> Self {
        Self {
            samples: [EMPTY; FRAME_HISTORY],
            next: 0,
            len: 0,
            last_start: None,
        }
    }

    /// Stores a frame that started at `start`; returns the sample.
    fn record(&mut self, frame: u64, start: Instant, render_us: u32, paint_us: u32) -> FrameSample {
        let interval = self.last_start.map_or(std::time::Duration::ZERO, |prev| {
            start.saturating_duration_since(prev)
        });
        self.last_start = Some(start);
        let sample = FrameSample {
            frame,
            render_us,
            paint_us,
            interval_us: micros(interval),
        };
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % FRAME_HISTORY;
        self.len = (self.len + 1).min(FRAME_HISTORY);
        sample
    }

    /// Oldest first.
    fn recent(&self) -> Vec<FrameSample> {
        assert!(self.len <= FRAME_HISTORY && self.next < FRAME_HISTORY);
        if self.len < FRAME_HISTORY {
            return self.samples[..self.len].to_vec();
        }
        let (newer, older) = self.samples.split_at(self.next);
        [older, newer].concat()
    }
}

fn micros(d: std::time::Duration) -> u32 {
    u32::try_from(d.as_micros()).unwrap_or(u32::MAX)
}

/// Frame number of the editor paint in progress or last finished.
pub(crate) fn current() -> u64 {
    FRAME.load(Ordering::Relaxed)
}

/// A clock read, or `None` when the debug server is off.
pub(crate) fn now() -> Option<Instant> {
    super::enabled().then(Instant::now)
}

pub(crate) fn micros_since(start: Option<Instant>) -> u32 {
    start.map_or(0, |s| micros(s.elapsed()))
}

pub(crate) struct FrameClock {
    frame: u64,
    start: Instant,
}

/// Starts timing one editor paint and numbers the frame.
pub(crate) fn start() -> Option<FrameClock> {
    super::enabled().then(|| FrameClock {
        frame: FRAME.fetch_add(1, Ordering::Relaxed) + 1,
        start: Instant::now(),
    })
}

/// Records the frame; `render_us` is the time spent building it.
pub(crate) fn finish(clock: Option<FrameClock>, render_us: u32) {
    let Some(clock) = clock else {
        return;
    };
    let paint_us = micros_since(Some(clock.start));
    let sample = match HISTORY.lock() {
        Ok(mut history) => history.record(clock.frame, clock.start, render_us, paint_us),
        Err(_) => return,
    };
    if super::wants(Topic::Frames) {
        super::emit(&Event::Frames(sample));
    }
}

pub(crate) fn recent() -> Vec<FrameSample> {
    HISTORY.lock().map(|h| h.recent()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_ring_keeps_the_newest_frames_oldest_first() {
        let mut history = History::new();
        let t0 = Instant::now();
        assert!(history.recent().is_empty());
        for frame in 1..=(FRAME_HISTORY as u64 + 10) {
            history.record(frame, t0, 1, 2);
        }
        let recent = history.recent();
        assert_eq!(recent.len(), FRAME_HISTORY);
        assert_eq!(recent[0].frame, 11);
        assert_eq!(recent[FRAME_HISTORY - 1].frame, FRAME_HISTORY as u64 + 10);
        assert!(recent.windows(2).all(|w| w[0].frame + 1 == w[1].frame));
    }

    #[test]
    fn a_partly_filled_ring_returns_only_what_was_recorded() {
        let mut history = History::new();
        let t0 = Instant::now();
        history.record(1, t0, 5, 6);
        history.record(2, t0 + Duration::from_millis(16), 7, 8);
        let recent = history.recent();
        assert_eq!(recent.iter().map(|s| s.frame).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(recent[0].interval_us, 0, "no previous frame");
        assert_eq!(recent[1].interval_us, 16_000);
        assert_eq!((recent[1].render_us, recent[1].paint_us), (7, 8));
    }

    #[test]
    fn timing_a_frame_costs_far_less_than_the_frame_budget() {
        const FRAMES: u32 = 100_000;
        // SAFETY: tests run single-threaded (`--test-threads=1`).
        unsafe { std::env::set_var("JADE_DEBUG_SOCKET", "/nonexistent/jade-bench.sock") };
        let started = Instant::now();
        for _ in 0..FRAMES {
            let render = now();
            let render_us = micros_since(render);
            finish(start(), render_us);
        }
        let per_frame = started.elapsed() / FRAMES;
        unsafe { std::env::remove_var("JADE_DEBUG_SOCKET") };
        eprintln!("timing_a_frame_costs_far_less_than_the_frame_budget: {per_frame:?}/frame");
        // This build re-reads the env var on every `enabled()`; release caches it.
        assert!(
            per_frame < std::time::Duration::from_micros(20),
            "{per_frame:?}"
        );
    }

    #[test]
    fn durations_beyond_u32_saturate() {
        assert_eq!(micros(Duration::from_secs(10_000)), u32::MAX);
    }
}
