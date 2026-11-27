//! Opt-in per-frame record of what the editor canvas painted, sent over the
//! debug socket (`JADE_DEBUG_SOCKET`, see `debug_server/`). It exists
//! because some rendering bugs fix themselves on the next frame (a stale
//! height, say), so a screenshot or an on-demand dump only ever sees the
//! good frame. A frame is emitted only when it differs from the last one
//! emitted, so an idle app is silent. Each trace starts `ok` or
//! `INCONSISTENT(...)`.

use std::fmt::Write as _;
use std::sync::Mutex;

use super::preview::callout::CalloutType;

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

/// A box painted behind lines from the row cache's positions (a callout's tint
/// or a table's grid), which must line up with where the lines went.
struct Band {
    start_line: usize,
    end_line: usize,
    top: f32,
    bottom: f32,
    /// `"callout"` or `"table"`.
    what: &'static str,
    /// The callout's type, empty for a table.
    kind: String,
}

/// Where one columns region put its parts, in pixels below the editor's top
/// (x from its left edge), so overlaps can be told from the numbers.
pub(super) struct RegionGeom {
    pub start_line: usize,
    pub top: f32,
    pub height: f32,
    pub columns: Vec<ColumnGeom>,
}

pub(super) struct ColumnGeom {
    pub x: f32,
    pub width: f32,
    /// Where the last line ends (the column's content bottom).
    pub bottom: f32,
    /// `(line, y, rows, height)`, capped at [`MAX_COLUMN_LINES`].
    pub lines: Vec<(usize, f32, usize, f32)>,
    /// A table box: `(first line, y, width, height)`.
    pub tables: Vec<(usize, f32, f32, f32)>,
}

const MAX_COLUMN_LINES: usize = 40;

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
    regions: Vec<RegionGeom>,
    elided_bands: usize,
    elided_lines: usize,
}

impl PaintTrace {
    pub(super) fn start_if_enabled(info: impl FnOnce() -> FrameInfo) -> Option<Self> {
        enabled().then(|| Self {
            info: info(),
            late_corrections_after: 0,
            bands: Vec::new(),
            regions: Vec::new(),
            lines: Vec::new(),
            elided_bands: 0,
            elided_lines: 0,
        })
    }

    pub(super) fn region(&mut self, mut region: RegionGeom) {
        for column in &mut region.columns {
            column.lines.truncate(MAX_COLUMN_LINES);
        }
        if self.regions.len() < MAX_BANDS {
            self.regions.push(region);
        }
    }

    pub(super) fn band(
        &mut self,
        lines: std::ops::Range<usize>,
        (top, bottom): (f32, f32),
        kind: CalloutType,
    ) {
        self.push_band(lines, (top, bottom), "callout", format!("{kind:?}"));
    }

    /// A table's box: `lines` is its first line to one past its last.
    pub(super) fn table_band(&mut self, lines: std::ops::Range<usize>, (top, bottom): (f32, f32)) {
        self.push_band(lines, (top, bottom), "table", String::new());
    }

    fn push_band(
        &mut self,
        lines: std::ops::Range<usize>,
        (top, bottom): (f32, f32),
        what: &'static str,
        kind: String,
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
            what,
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
                "{late} line heights corrected after the callout bands and table boxes were placed"
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
                    "{} at line {} starts at y={:.1} but its first line is at y={:.1}",
                    band.what, band.start_line, band.top, first.y
                ));
            }
            if let Some(last) = last
                && last.index + 1 == band.end_line
                && ((last.y + last.height) - band.bottom).abs() > TOLERANCE
            {
                problems.push(format!(
                    "{} ending at line {} ends at y={:.1} but its last line ends at y={:.1}",
                    band.what,
                    band.end_line,
                    band.bottom,
                    last.y + last.height
                ));
            }
        }
        for region in &self.regions {
            region_problems(region, &mut problems);
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
                " | band {} lines {}..{} y {:.1}..{:.1}",
                if b.what == "table" { "table" } else { &b.kind },
                b.start_line,
                b.end_line,
                b.top,
                b.bottom
            );
        }
        for l in &self.lines {
            let _ = write!(
                s,
                " | L{}@{:.1} rows={} h={:.1} {} {:?}",
                l.index, l.y, l.rows, l.height, l.tag, l.preview
            );
        }
        for r in &self.regions {
            let _ = write!(
                s,
                " | region L{} y {:.1} h {:.1}",
                r.start_line, r.top, r.height
            );
            for (k, c) in r.columns.iter().enumerate() {
                let _ = write!(
                    s,
                    " | col{k} x {:.1} w {:.1} bottom {:.1} lines",
                    c.x, c.width, c.bottom
                );
                for (line, y, rows, _) in &c.lines {
                    let _ = write!(s, " {line}@{y:.1}/{rows}");
                }
                for (line, y, w, h) in &c.tables {
                    let _ = write!(s, " table L{line} y {y:.1} w {w:.1} h {h:.1}");
                }
            }
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

/// What is wrong with how a region's parts sit: anything that would paint over
/// a neighbour or past the region's own height.
fn region_problems(r: &RegionGeom, problems: &mut Vec<String>) {
    let region_bottom = r.top + r.height;
    for (k, c) in r.columns.iter().enumerate() {
        if c.bottom > region_bottom + TOLERANCE {
            problems.push(format!(
                "region at line {}: column {k} ends at y={:.1}, below the region's bottom {:.1}",
                r.start_line, c.bottom, region_bottom
            ));
        }
        for pair in c.lines.windows(2) {
            let ((a, ay, _, ah), (b, by, _, _)) = (pair[0], pair[1]);
            if by + TOLERANCE < ay + ah {
                problems.push(format!(
                    "region at line {}: column {k} line {b} (y={by:.1}) starts inside line {a} (y={ay:.1}, h={ah:.1})",
                    r.start_line
                ));
            }
        }
        for &(line, y, w, h) in &c.tables {
            if w > c.width + TOLERANCE {
                problems.push(format!(
                    "region at line {}: the table at line {line} in column {k} is {w:.1} wide in a {:.1} column",
                    r.start_line, c.width
                ));
            }
            if y + h > c.bottom + TOLERANCE {
                problems.push(format!(
                    "region at line {}: the table at line {line} in column {k} ends at y={:.1}, below its column's {:.1}",
                    r.start_line,
                    y + h,
                    c.bottom
                ));
            }
        }
    }
    // Floats stacked down one side share an x range; they only collide if their
    // rows do too.
    let top = |c: &ColumnGeom| c.lines.iter().map(|l| l.1).fold(f32::INFINITY, f32::min);
    for (i, a) in r.columns.iter().enumerate() {
        for b in &r.columns[i + 1..] {
            let sideways = a.x < b.x + b.width - TOLERANCE && b.x < a.x + a.width - TOLERANCE;
            let down = top(a) < b.bottom - TOLERANCE && top(b) < a.bottom - TOLERANCE;
            if sideways && down {
                problems.push(format!(
                    "region at line {}: columns overlap sideways",
                    r.start_line
                ));
            }
        }
    }
}

#[cfg(test)]
static LAST_FRAME: Mutex<PaintedFrame> = Mutex::new(PaintedFrame::new());

/// Where the last traced frame put its lines, for the acceptance catalog.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct PaintedFrame {
    pub problems: Vec<String>,
    /// Top-level lines in paint order, then those of side-by-side regions.
    pub lines: Vec<PaintedLine>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct PaintedLine {
    pub line: usize,
    /// Window y.
    pub y: f32,
    pub height: f32,
    pub in_region: bool,
}

#[cfg(test)]
impl PaintedFrame {
    const fn new() -> Self {
        Self {
            problems: Vec::new(),
            lines: Vec::new(),
        }
    }

    pub(crate) fn y_of(&self, line: usize) -> Option<f32> {
        self.lines.iter().find(|l| l.line == line).map(|l| l.y)
    }
}

#[cfg(test)]
pub(crate) fn last_painted_frame() -> PaintedFrame {
    LAST_FRAME.lock().map(|f| f.clone()).unwrap_or_default()
}

#[cfg(test)]
impl PaintTrace {
    fn painted(&self, problems: &[String]) -> PaintedFrame {
        let editor_top = self.info.bounds.1;
        let top_level = self.lines.iter().map(|l| PaintedLine {
            line: l.index,
            y: l.y,
            height: l.height,
            in_region: false,
        });
        let columns = self.regions.iter().flat_map(|r| &r.columns);
        let in_regions = columns
            .flat_map(|c| &c.lines)
            .map(|&(line, y, _, height)| PaintedLine {
                line,
                y: editor_top + y,
                height,
                in_region: true,
            });
        PaintedFrame {
            problems: problems.to_vec(),
            lines: top_level.chain(in_regions).collect(),
        }
    }
}

impl Drop for PaintTrace {
    fn drop(&mut self) {
        let frame = crate::debug_server::frames::current();
        let problems = self.problems();
        #[cfg(test)]
        if let Ok(mut painted) = LAST_FRAME.lock() {
            *painted = self.painted(&problems);
        }
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
            regions: Vec::new(),
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
        t.band(1..3, (20., 60.), CalloutType::Note);
        assert!(t.problems().is_empty());
        assert!(t.signature(&[]).starts_with("ok "));
    }

    #[test]
    fn a_band_that_stops_short_is_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.band(1..3, (20., 40.), CalloutType::Note);
        let problems = t.problems();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("ends at y=40.0"), "{problems:?}");
    }

    #[test]
    fn a_band_that_starts_off_is_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.band(1..3, (10., 60.), CalloutType::Note);
        assert!(t.problems()[0].contains("starts at y=10.0"));
    }

    #[test]
    fn a_table_box_off_its_lines_is_reported() {
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.table_band(1..3, (20., 60.));
        assert!(t.problems().is_empty());
        let mut t = trace();
        lay_out_four_lines(&mut t);
        t.table_band(1..3, (33., 73.));
        let problems = t.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("table at line 1 starts at y=33.0"));
        assert!(problems[1].starts_with("table ending at line 3 ends at y=73.0"));
        assert!(
            t.signature(&problems)
                .contains(" | band table lines 1..3 y 33.0..73.0")
        );
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
        t.band(2..4, (40., 80.), CalloutType::Tip);
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
        t.band(0..10, (0., 200.), CalloutType::Note);
        let sig = t.signature(&[]);
        assert!(sig.len() < 20_000, "{} bytes", sig.len());
        assert!(sig.contains("elided 0 bands, 9850 lines"));
    }

    fn column(x: f32, width: f32, bottom: f32) -> ColumnGeom {
        ColumnGeom {
            x,
            width,
            bottom,
            lines: vec![(4, 100., 2, 52.), (6, 152., 1, 26.)],
            tables: Vec::new(),
        }
    }

    #[test]
    fn a_region_whose_parts_fit_is_consistent() {
        let mut t = trace();
        t.region(RegionGeom {
            start_line: 2,
            top: 100.,
            height: 100.,
            columns: vec![column(0., 200., 178.), column(224., 300., 178.)],
        });
        assert!(t.problems().is_empty(), "{:?}", t.problems());
        assert!(
            t.signature(&[])
                .contains("| region L2 y 100.0 h 100.0 | col0 x 0.0")
        );
    }

    #[test]
    fn overlaps_in_a_region_are_reported() {
        let mut t = trace();
        let mut wide = column(224., 300., 178.);
        wide.tables.push((5, 100., 340., 30.));
        let mut stacked = column(0., 230., 260.);
        stacked.lines[1].1 = 120.;
        t.region(RegionGeom {
            start_line: 2,
            top: 100.,
            height: 100.,
            columns: vec![stacked, wide],
        });
        let problems = t.problems().join("\n");
        for needle in [
            "below the region's bottom",
            "starts inside line",
            "340.0 wide",
            "overlap sideways",
        ] {
            assert!(
                problems.contains(needle),
                "missing {needle:?} in {problems}"
            );
        }
    }
}
