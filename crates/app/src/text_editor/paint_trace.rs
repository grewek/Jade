//! Opt-in per-frame record of what the editor canvas painted, sent over the
//! debug socket (`JADE_DEBUG_SOCKET`, see `debug_server/`). It exists
//! because some rendering bugs fix themselves on the next frame (a stale
//! height, say), so a screenshot or an on-demand dump only ever sees the
//! good frame. A frame is emitted only when it differs from the last one
//! emitted, so an idle app is silent. Each trace starts `ok` or
//! `INCONSISTENT(...)`.

use std::fmt::Write as _;
use std::sync::Mutex;

use super::callout::CalloutType;

/// Pixel disagreement below this is rounding, not a bug.
const TOLERANCE: f32 = 0.5;
/// Enough to recognise a line in the note without bloating the frame line.
const PREVIEW_CHARS: usize = 30;
/// A frame whose window is huge (a zero-height viewport paints the whole
/// document) must not become a multi-megabyte socket line: only the first
/// lines and bands are kept, and the counts of the rest are reported.
const MAX_LINES: usize = 150;
const MAX_BANDS: usize = 60;

static LAST_SIGNATURE: Mutex<String> = Mutex::new(String::new());

pub(super) fn enabled() -> bool {
    crate::debug_server::enabled()
}

/// What the canvas knew about the frame before painting any line.
pub(super) struct FrameInfo {
    pub bounds: (f32, f32, f32, f32),
    pub viewport: (f32, f32, f32, f32),
    pub cursor: usize,
    pub focused: bool,
    pub range: std::ops::Range<usize>,
    pub total_height: f32,
    /// The wrap width the row heights were computed for; `None` while still
    /// unknown (the first frame).
    pub cached_width: Option<f32>,
    pub verified: std::ops::Range<usize>,
    pub late_corrections_before: usize,
}

struct Band {
    start_line: usize,
    end_line: usize,
    top: f32,
    bottom: f32,
    kind: CalloutType,
}

struct Line {
    index: usize,
    y: f32,
    rows: usize,
    height: f32,
    tag: &'static str,
    preview: String,
}

/// Collects one frame; emits (if new) when dropped.
pub(super) struct PaintTrace {
    info: FrameInfo,
    late_corrections_after: usize,
    bands: Vec<Band>,
    lines: Vec<Line>,
    elided_bands: usize,
    elided_lines: usize,
}

impl PaintTrace {
    pub(super) fn start_if_enabled(info: impl FnOnce() -> FrameInfo) -> Option<Self> {
        enabled().then(|| Self {
            info: info(),
            late_corrections_after: 0,
            bands: Vec::new(),
            lines: Vec::new(),
            elided_bands: 0,
            elided_lines: 0,
        })
    }

    pub(super) fn band(
        &mut self,
        lines: std::ops::Range<usize>,
        top: f32,
        bottom: f32,
        kind: CalloutType,
    ) {
        if self.bands.len() >= MAX_BANDS {
            self.elided_bands += 1;
            return;
        }
        self.bands.push(Band {
            start_line: lines.start,
            end_line: lines.end,
            top,
            bottom,
            kind,
        });
    }

    pub(super) fn line(
        &mut self,
        index: usize,
        y: f32,
        rows: usize,
        height: f32,
        tag: &'static str,
        text: &str,
    ) {
        if self.lines.len() >= MAX_LINES {
            self.elided_lines += 1;
            return;
        }
        self.lines.push(Line {
            index,
            y,
            rows,
            height,
            tag,
            preview: text.chars().take(PREVIEW_CHARS).collect(),
        });
    }

    pub(super) fn set_late_corrections(&mut self, total: usize) {
        self.late_corrections_after = total;
    }

    /// Everything that is wrong with this frame, empty when consistent.
    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let late = self
            .late_corrections_after
            .saturating_sub(self.info.late_corrections_before);
        if late > 0 {
            problems.push(format!(
                "{late} line heights corrected after the callout bands were placed"
            ));
        }
        for band in &self.bands {
            let in_band = |l: &&Line| l.index >= band.start_line && l.index < band.end_line;
            let first = self.lines.iter().find(in_band);
            let last = self.lines.iter().rfind(in_band);
            if let Some(first) = first
                && first.index == band.start_line
                && (first.y - band.top).abs() > TOLERANCE
            {
                problems.push(format!(
                    "callout at line {} starts at y={:.1} but its first line is at y={:.1}",
                    band.start_line, band.top, first.y
                ));
            }
            if let Some(last) = last
                && last.index + 1 == band.end_line
                && ((last.y + last.height) - band.bottom).abs() > TOLERANCE
            {
                problems.push(format!(
                    "callout ending at line {} ends at y={:.1} but its last line ends at y={:.1}",
                    band.end_line,
                    band.bottom,
                    last.y + last.height
                ));
            }
        }
        problems
    }

    /// The frame as one line, without the frame number (so two frames with
    /// identical content compare equal).
    fn signature(&self, problems: &[String]) -> String {
        let i = &self.info;
        let mut s = String::new();
        if problems.is_empty() {
            s.push_str("ok");
        } else {
            let _ = write!(s, "INCONSISTENT({})", problems.join("; "));
        }
        let width = i
            .cached_width
            .map_or("unknown".to_string(), |w| format!("{w:.0}"));
        let (bx, by, bw, bh) = i.bounds;
        let (vx, vy, vw, vh) = i.viewport;
        let _ = write!(
            s,
            " bounds=({bx:.0},{by:.0} {bw:.0}x{bh:.0}) viewport=({vx:.0},{vy:.0} {vw:.0}x{vh:.0}) \
             cursor={} focused={} range={}..{} total_h={:.0} wrap_width={width} verified={}..{}",
            i.cursor,
            i.focused,
            i.range.start,
            i.range.end,
            i.total_height,
            i.verified.start,
            i.verified.end,
        );
        for b in &self.bands {
            let _ = write!(
                s,
                " | band {:?} lines {}..{} y {:.1}..{:.1}",
                b.kind, b.start_line, b.end_line, b.top, b.bottom
            );
        }
        for l in &self.lines {
            let _ = write!(
                s,
                " | L{}@{:.1} rows={} {} {:?}",
                l.index, l.y, l.rows, l.tag, l.preview
            );
        }
        if self.elided_bands + self.elided_lines > 0 {
            let _ = write!(
                s,
                " | elided {} bands, {} lines",
                self.elided_bands, self.elided_lines
            );
        }
        s
    }
}

impl Drop for PaintTrace {
    fn drop(&mut self) {
        let frame = crate::debug_server::frames::current();
        let problems = self.problems();
        let signature = self.signature(&problems);
        let Ok(mut last) = LAST_SIGNATURE.lock() else {
            return;
        };
        if *last == signature {
            return;
        }
        crate::debug_server::emit(&jade_debug_protocol::Event::Paint {
            frame,
            consistent: problems.is_empty(),
            trace: signature.clone(),
        });
        *last = signature;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace() -> PaintTrace {
        PaintTrace {
            info: FrameInfo {
                bounds: (0., 0., 800., 600.),
                viewport: (0., 0., 800., 600.),
                cursor: 0,
                focused: true,
                range: 0..4,
                total_height: 80.,
                cached_width: None,
                verified: 0..4,
                late_corrections_before: 0,
            },
            late_corrections_after: 0,
            bands: Vec::new(),
            lines: Vec::new(),
            elided_bands: 0,
            elided_lines: 0,
        }
    }

    fn lay_out_four_lines(t: &mut PaintTrace) {
        for i in 0..4 {
            t.line(i, i as f32 * 20., 1, 20., "plain", "text");
        }
    }

    #[test]
    fn a_band_matching_its_lines_is_consistent() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.band(1..3, 20., 60., CalloutType::Note);
        assert!(t.problems().is_empty());
        assert!(t.signature(&[]).starts_with("ok "));
    }

    #[test]
    fn a_band_that_stops_short_is_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.band(1..3, 20., 40., CalloutType::Note);
        let problems = t.problems();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("ends at y=40.0"), "{problems:?}");
    }

    #[test]
    fn a_band_that_starts_off_is_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.band(1..3, 10., 60., CalloutType::Note);
        assert!(t.problems()[0].contains("starts at y=10.0"));
    }

    #[test]
    fn late_height_corrections_are_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.set_late_corrections(2);
        assert!(t.problems()[0].contains("2 line heights corrected"));
    }

    #[test]
    fn a_band_running_past_the_window_is_only_checked_where_visible() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        // Band covers lines 2..9; only 2 and 3 were painted, its recorded
        // bottom is the window's end.
        t.band(2..4, 40., 80., CalloutType::Tip);
        assert!(t.problems().is_empty());
    }

    #[test]
    fn identical_frames_share_a_signature_and_unknown_width_is_visible() {
        let mut a = trace();
        lay_out_four_lines(&mut a);
        let mut b = trace();
        lay_out_four_lines(&mut b);
        assert_eq!(a.signature(&[]), b.signature(&[]));
        assert!(a.signature(&[]).contains("wrap_width=unknown"));
    }

    #[test]
    fn a_huge_window_is_truncated_not_sent_whole() {
        let mut t = trace();
        for i in 0..10_000 {
            t.line(i, i as f32 * 20., 1, 20., "plain", "text");
        }
        t.band(0..10, 0., 200., CalloutType::Note);
        let sig = t.signature(&[]);
        assert!(sig.len() < 20_000, "{} bytes", sig.len());
        assert!(sig.contains("elided 0 bands, 9850 lines"));
    }
}
