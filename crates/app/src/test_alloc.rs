//! A counting `GlobalAlloc` wrapper, `#[cfg(test)]`-only — lets a test
//! measure actual heap allocation behavior (live bytes, peak bytes,
//! allocation *count*) directly, rather than inferring it from OS-level RSS
//! (`btop`/`/proc/self/status`), which is affected by the allocator's own
//! arena/mmap-threshold heuristics and doesn't distinguish "freed but not
//! yet returned to the OS" from "still actually live." Zero effect on
//! non-test builds (release/`cargo run`) — the `#[global_allocator]`
//! override in `main.rs` is itself `#[cfg(test)]`-gated.
//!
//! Added to investigate a real user report: `jade-app` using ~189MB at
//! startup and ~3xxMB with the 320K-line stress fixture open, well above
//! what the document's own text (~11.3MB) plus `DocumentCache`/
//! `RowHeightCache`'s per-line arrays (~45MB, estimated from their actual
//! field types) should cost. See `document_cache.rs`'s and `main.rs`'s own
//! tests that use this for what it found.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Delegates every call straight to [`System`], just adding atomic
/// bookkeeping around each one — no behavioral difference from the plain
/// system allocator a test build would otherwise use.
pub(crate) struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_grow(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        record_shrink(layout.size());
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_grow(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                record_grow(new_size - layout.size());
            } else {
                record_shrink(layout.size() - new_size);
            }
        }
        new_ptr
    }
}

fn record_grow(added: usize) {
    // Saturating, not a plain `fetch_add` — defense in depth against any
    // allocation this module's bookkeeping doesn't perfectly pair with a
    // matching `alloc`/`dealloc` (e.g. a buffer a native/FFI dependency
    // allocated outside Rust's `GlobalAlloc` entirely, later freed through
    // it): the root cause of a real crash this module caused (see
    // `reset`'s own doc comment) is fixed, but saturating here means even
    // an *unknown* remaining mismatch stays a slightly-off number, not a
    // panic that takes the whole test suite down.
    let mut current = CURRENT_BYTES.load(Ordering::Relaxed);
    let new_current = loop {
        let new = current.saturating_add(added);
        match CURRENT_BYTES.compare_exchange_weak(
            current,
            new,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break new,
            Err(actual) => current = actual,
        }
    };
    PEAK_BYTES.fetch_max(new_current, Ordering::Relaxed);
    ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn record_shrink(removed: usize) {
    let mut current = CURRENT_BYTES.load(Ordering::Relaxed);
    loop {
        let new = current.saturating_sub(removed);
        match CURRENT_BYTES.compare_exchange_weak(
            current,
            new,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

/// A point-in-time reading — every test using this module calls
/// [`snapshot`] before and after the operation it's measuring, then reports
/// the *difference* (peak/count only ever grow within a `reset`-to-
/// `snapshot` window, so a plain subtraction is always meaningful; `current`
/// is the one field worth reading absolutely, not diffed, since it answers
/// "how much is still actually live right now").
#[derive(Debug, Clone, Copy)]
pub(crate) struct AllocSnapshot {
    pub(crate) current: usize,
    pub(crate) peak: usize,
    pub(crate) count: usize,
}

/// Rebases [`PEAK_BYTES`] to whatever's *actually* currently live, and
/// zeroes [`ALLOC_COUNT`] — call this before the operation under test, so
/// `snapshot().peak`/`.count` read right after reflect only what happened
/// since. Deliberately does **not** touch [`CURRENT_BYTES`]: that one has
/// to stay a true, continuously-accurate running total since process
/// start, matching every `alloc`/`dealloc` pair exactly — forcing it to `0`
/// here (an earlier version of this function did) desyncs it from the
/// *real* live byte count the moment anything already-live (e.g. a
/// previous test's own retained allocations, in the same long-running test
/// binary) later gets freed, underflowing it (`fetch_sub` wrapping a `usize`
/// below zero) and then panicking the next `alloc` with "attempt to add
/// with overflow" in a debug build — confirmed exactly this way, not
/// guessed, when `cargo test`'s full suite (but not any one test run in
/// isolation) crashed after this module was added. Every caller already
/// reads `current` as a plain delta (`after.current - before.current`) for
/// exactly this reason, not as an absolute value assumed to start at `0`.
pub(crate) fn reset() {
    PEAK_BYTES.store(CURRENT_BYTES.load(Ordering::Relaxed), Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
}

pub(crate) fn snapshot() -> AllocSnapshot {
    AllocSnapshot {
        current: CURRENT_BYTES.load(Ordering::Relaxed),
        peak: PEAK_BYTES.load(Ordering::Relaxed),
        count: ALLOC_COUNT.load(Ordering::Relaxed),
    }
}
