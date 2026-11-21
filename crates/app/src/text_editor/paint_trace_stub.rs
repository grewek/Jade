//! No-op stand-in for `paint_trace.rs` in builds without the `debug-socket`
//! feature: `start_if_enabled` never starts a trace, so the other methods
//! are unreachable and exist only to keep `element.rs` free of `#[cfg]`.

use super::preview::callout::CalloutType;

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

/// Built only when tracing, which never happens here.
#[allow(dead_code)]
pub(super) struct RegionGeom {
    pub start_line: usize,
    pub top: f32,
    pub height: f32,
    pub columns: Vec<ColumnGeom>,
}

#[allow(dead_code)]
pub(super) struct ColumnGeom {
    pub x: f32,
    pub width: f32,
    pub bottom: f32,
    pub lines: Vec<(usize, f32, usize, f32)>,
    pub tables: Vec<(usize, f32, f32, f32)>,
}

pub(super) struct PaintTrace;

impl PaintTrace {
    pub(super) fn start_if_enabled(_info: impl FnOnce() -> FrameInfo) -> Option<Self> {
        None
    }

    pub(super) fn band(&mut self, _: std::ops::Range<usize>, _: f32, _: f32, _: CalloutType) {}

    pub(super) fn line(&mut self, _: usize, _: f32, _: usize, _: f32, _: &'static str, _: &str) {}

    pub(super) fn region(&mut self, _: RegionGeom) {}

    pub(super) fn set_late_corrections(&mut self, _total: usize) {}
}
