//! No-op stand-in for `paint_trace.rs` in builds without the `debug-socket`
//! feature: `start_if_enabled` never starts a trace, so the other methods
//! are unreachable and exist only to keep `element.rs` free of `#[cfg]`.

use super::callout::CalloutType;

/// Built only by the caller's closure, which never runs here.
#[allow(dead_code)]
pub(super) struct FrameInfo {
    pub bounds: (f32, f32, f32, f32),
    pub viewport: (f32, f32, f32, f32),
    pub cursor: usize,
    pub focused: bool,
    pub range: std::ops::Range<usize>,
    pub total_height: f32,
    pub cached_width: Option<f32>,
    pub verified: std::ops::Range<usize>,
    pub late_corrections_before: usize,
}

pub(super) struct PaintTrace;

impl PaintTrace {
    pub(super) fn start_if_enabled(_info: impl FnOnce() -> FrameInfo) -> Option<Self> {
        None
    }

    pub(super) fn band(&mut self, _: std::ops::Range<usize>, _: f32, _: f32, _: CalloutType) {}

    pub(super) fn line(&mut self, _: usize, _: f32, _: usize, _: f32, _: &'static str, _: &str) {}

    pub(super) fn set_late_corrections(&mut self, _total: usize) {}
}
