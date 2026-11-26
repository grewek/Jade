//! The block tree every candidate produces, and the one incremental driver
//! they share (#130 §6.5: reparse from the enclosing block until the result
//! matches the old tree).

use std::ops::ControlFlow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading,
    List,
    Item,
    Quote,
    Callout,
    Code,
    Math,
    Table,
    Rule,
    Html,
    Image,
    RefDef,
    Other,
    OpenAside,
    CloseAside,
    OpenColumns,
    CloseColumns,
    OpenColumn,
    CloseColumn,
}

/// The container a unit starts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    Top,
    Aside,
    Columns,
    Column,
}

/// A block nested in a unit; `start` is relative to the unit, so shifting
/// units after an edit never touches nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub depth: u8,
    pub start: u32,
    pub len: u32,
}

/// A block directly in the document or in a container, or a container tag
/// line. The tree is the flat list of these: containers are the spans between
/// their tag units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub start: u32,
    pub len: u32,
    pub kind: Kind,
    pub ctx: Ctx,
    pub children: Vec<Node>,
}

impl Unit {
    pub fn end(&self) -> usize {
        (self.start + self.len) as usize
    }
}

pub type Emit<'a> = dyn FnMut(Unit) -> ControlFlow<()> + 'a;

pub trait BlockParser {
    /// Emits the units of `text` from `from` (a unit boundary in container
    /// `ctx`) in order, until `emit` breaks or the text ends.
    fn parse_from(&mut self, text: &str, from: usize, ctx: Ctx, emit: &mut Emit<'_>);

    fn parse(&mut self, text: &str) -> Vec<Unit> {
        let mut units = Vec::new();
        self.parse_from(text, 0, Ctx::Top, &mut |unit| {
            units.push(unit);
            ControlFlow::Continue(())
        });
        units
    }
}

/// `removed` bytes at `at` were replaced by `inserted` bytes.
#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub at: usize,
    pub removed: usize,
    pub inserted: usize,
}

/// How far above an edit a `$$` opener can be and still change meaning
/// (`math_scan::MAX_BLOCK_LINES`).
const MATH_REACH: usize = jade_core::math_scan::MAX_BLOCK_LINES;

/// The first unit to reparse for an edit at `at`: the one before the unit the
/// edit is in (it may absorb it: lazy line, setext underline, table
/// delimiter), or an earlier one with a `$$` the edit may now close.
fn restart_unit(units: &[Unit], text: &str, at: usize) -> usize {
    let touched = units.partition_point(|u| u.end() < at);
    let mut first = touched.saturating_sub(1);
    // Units before `first` are untouched, so their offsets are valid in the
    // new text.
    let (mut i, mut lines) = (first, 0);
    while i > 0 && lines <= MATH_REACH {
        i -= 1;
        let unit = &units[i];
        let body = &text.as_bytes()[unit.start as usize..unit.end()];
        lines += 1 + body.iter().filter(|&&b| b == b'\n').count();
        // Only a `$$` that opened nothing can still change; a closed block
        // cannot.
        if unit.kind != Kind::Math
            && body.windows(2).any(|w| w == b"$$")
            && text[unit.start as usize..unit.end()]
                .lines()
                .any(jade_core::math_scan::opens_block)
        {
            first = i;
        }
    }
    first
}

/// Statistics of one update, for the report.
#[derive(Default, Clone, Copy)]
pub struct Updated {
    pub reparsed_units: usize,
    pub resynced: bool,
}

/// Updates `units` (the tree of the text before `edit`) to the tree of
/// `text` (after it).
pub fn update(
    parser: &mut dyn BlockParser,
    units: &mut Vec<Unit>,
    fresh: &mut Vec<Unit>,
    text: &str,
    edit: Edit,
) -> Updated {
    let first = if units.is_empty() {
        0
    } else {
        restart_unit(units, text, edit.at)
    };
    let (from, ctx) = match units.get(first) {
        Some(u) if first > 0 => (u.start as usize, u.ctx),
        _ => (0, Ctx::Top),
    };
    let delta = edit.inserted as isize - edit.removed as isize;
    let unchanged_after = edit.at + edit.inserted;
    fresh.clear();
    let mut old = first;
    let mut resynced = false;
    parser.parse_from(text, from, ctx, &mut |unit| {
        // Strictly after the edit, so the line break before it is old too.
        if unit.start as usize > unchanged_after {
            let was = (unit.start as isize - delta) as usize;
            while old < units.len() && (units[old].start as usize) < was {
                old += 1;
            }
            if units.get(old).is_some_and(|o| {
                o.start as usize == was
                    && o.len == unit.len
                    && o.kind == unit.kind
                    && o.ctx == unit.ctx
            }) {
                resynced = true;
                return ControlFlow::Break(());
            }
        }
        fresh.push(unit);
        ControlFlow::Continue(())
    });
    if !resynced {
        old = units.len();
    }
    let reparsed_units = fresh.len();
    let tail = first + reparsed_units;
    units.splice(first..old, fresh.drain(..));
    if delta != 0 {
        for unit in &mut units[tail..] {
            unit.start = (unit.start as isize + delta) as u32;
        }
    }
    Updated {
        reparsed_units,
        resynced,
    }
}

/// Whether `line` is the strict-form tag that is valid in `ctx`, and the
/// container after it. A closing line ends its container even inside code
/// (K10), so callers test closers before anything else.
pub fn tag_line(line: &str, ctx: Ctx) -> Option<(Kind, Ctx)> {
    if !line.starts_with('<') {
        return None;
    }
    match (ctx, line.trim_end()) {
        (Ctx::Top, "<jade-columns>") => Some((Kind::OpenColumns, Ctx::Columns)),
        (Ctx::Top, "<jade-aside side=\"left\">" | "<jade-aside side=\"right\">") => {
            Some((Kind::OpenAside, Ctx::Aside))
        }
        (Ctx::Aside, "</jade-aside>") => Some((Kind::CloseAside, Ctx::Top)),
        (Ctx::Columns, "<jade-column>") => Some((Kind::OpenColumn, Ctx::Column)),
        (Ctx::Columns | Ctx::Column, "</jade-columns>") => Some((Kind::CloseColumns, Ctx::Top)),
        (Ctx::Column, "</jade-column>") => Some((Kind::CloseColumn, Ctx::Columns)),
        _ => None,
    }
}

pub fn is_closer(line: &str, ctx: Ctx) -> bool {
    line.starts_with("</") && tag_line(line, ctx).is_some()
}

/// The line starting at `pos` and the offset of the next one.
pub fn line_at(text: &str, pos: usize) -> (&str, usize) {
    match text[pos..].find('\n') {
        Some(i) => (&text[pos..pos + i], pos + i + 1),
        None => (&text[pos..], text.len()),
    }
}

pub fn is_blank(line: &str) -> bool {
    line.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r'))
}

/// Moves a block's range to whole lines: from its first line's start to the
/// end of its last non-blank line. Parsers disagree on trailing blank lines.
pub fn whole_lines(text: &str, start: usize, end: usize) -> (usize, usize) {
    let start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let end = start + text[start..end].trim_end().len();
    let end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    (start, end)
}

/// A parser that only takes a whole string of Markdown, with no containers
/// and no way to stop: the libraries.
pub trait SegmentParser {
    /// Appends the units of `segment`, whose offset in the note is `base`.
    fn parse_segment(&mut self, segment: &str, base: usize, ctx: Ctx, out: &mut Vec<Unit>);
}

/// The pre-pass the libraries need: the next line from `scan` that reads as
/// a container tag, or where the window starting at `from` ends. Whether an
/// opening tag is code is not decided here: the parser itself is asked, so
/// the two cannot disagree about fences.
pub struct Boundary {
    pub end: usize,
    /// The tag line at `end`: its kind, the container after it, the line's end.
    pub tag: Option<(Kind, Ctx, usize)>,
    /// `end` is the segment's real end, not just the window's.
    pub complete: bool,
}

pub fn next_boundary(text: &str, from: usize, scan: usize, ctx: Ctx, window: usize) -> Boundary {
    let mut pos = scan;
    while pos < text.len() {
        let (line, next) = line_at(text, pos);
        if let Some((kind, after)) = tag_line(line, ctx) {
            let tag = Some((kind, after, pos + line.len()));
            return Boundary {
                end: pos,
                tag,
                complete: true,
            };
        }
        pos = next;
        if pos - from >= window && pos < text.len() {
            return Boundary {
                end: pos,
                tag: None,
                complete: false,
            };
        }
    }
    Boundary {
        end: text.len(),
        tag: None,
        complete: true,
    }
}

const FIRST_WINDOW: usize = 1024;
/// Units at a cut-off window's end that more text could still change.
const UNSAFE_TAIL: usize = 2;

/// How many units of a cut-off window more text cannot change: not the last
/// ones, nor anything from a `$$` that may find its closer past the cut.
fn reliable(text: &str, units: &[Unit], cut: usize) -> usize {
    let safe = units.len().saturating_sub(UNSAFE_TAIL);
    let open_math = units[..safe].iter().position(|u| {
        let body = &text.as_bytes()[u.start as usize..u.end()];
        u.kind != Kind::Math
            && body.windows(2).any(|w| w == b"$$")
            && text.as_bytes()[u.start as usize..cut]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
                <= MATH_REACH
    });
    open_math.unwrap_or(safe)
}

/// Gives a [`SegmentParser`] the streaming interface: parses growing windows
/// and emits only what more text cannot change.
pub struct Windowed<P> {
    pub inner: P,
    pub buffer: Vec<Unit>,
}

impl<P: SegmentParser> Windowed<P> {
    pub fn new(inner: P) -> Self {
        Windowed {
            inner,
            buffer: Vec::new(),
        }
    }
}

impl<P: SegmentParser> Windowed<P> {
    fn segment(&mut self, text: &str, from: usize, to: usize, ctx: Ctx) {
        self.inner
            .parse_segment(&text[from..to], from, ctx, &mut self.buffer);
    }

    /// Fills the buffer with the units from `from` up to the next real
    /// boundary or the window's end. An opening tag is real unless parsing
    /// through it shows it became code. While tags turn out to be code, the
    /// distance doubles before asking again, which keeps this linear when an
    /// open fence swallows many of them.
    fn fill(&mut self, text: &str, from: usize, ctx: Ctx, window: usize) -> Boundary {
        let (mut scan, mut probe_after) = (from, 0);
        let mut pending: Vec<Boundary> = Vec::new();
        loop {
            let boundary = next_boundary(text, from, scan, ctx, window);
            let tag_end = boundary.tag.map(|t| t.2);
            let closer = tag_end.is_some_and(|end| is_closer(&text[boundary.end..end], ctx));
            let to = tag_end.unwrap_or(boundary.end);
            if tag_end.is_some() && !closer && to - from < probe_after {
                scan = line_at(text, boundary.end).1;
                pending.push(boundary);
                continue;
            }
            self.buffer.clear();
            self.segment(text, from, if closer { boundary.end } else { to }, ctx);
            for candidate in pending.drain(..) {
                if self.cut_at_tag(text, ctx, candidate.end) {
                    return candidate;
                }
            }
            if tag_end.is_none() || closer || self.cut_at_tag(text, ctx, boundary.end) {
                return boundary;
            }
            probe_after = (to - from) * 2;
            scan = line_at(text, boundary.end).1;
        }
    }

    /// If the opening tag at `tag_at` is not code in the parsed buffer, cuts
    /// the buffer there.
    fn cut_at_tag(&mut self, text: &str, ctx: Ctx, tag_at: usize) -> bool {
        let i = self.buffer.partition_point(|u| u.end() <= tag_at);
        let Some(unit) = self.buffer.get(i) else {
            return true;
        };
        let (start, kind) = (unit.start as usize, unit.kind);
        if start < tag_at && matches!(kind, Kind::Code | Kind::Math) {
            return false;
        }
        // Redo only the block that had taken the tag's line.
        self.buffer.truncate(i);
        if start < tag_at {
            let start = text[..start].rfind('\n').map_or(0, |n| n + 1);
            self.segment(text, start, tag_at, ctx);
        }
        true
    }
}

impl<P: SegmentParser> BlockParser for Windowed<P> {
    fn parse_from(&mut self, text: &str, mut from: usize, mut ctx: Ctx, emit: &mut Emit<'_>) {
        let mut window = FIRST_WINDOW;
        while from < text.len() {
            let boundary = self.fill(text, from, ctx, window);
            let keep = match boundary.complete {
                true => self.buffer.len(),
                false => reliable(text, &self.buffer, boundary.end),
            };
            let mut reached = from;
            for unit in self.buffer.drain(..).take(keep) {
                reached = unit.end();
                if emit(unit).is_break() {
                    return;
                }
            }
            if !boundary.complete {
                if keep > 0 {
                    from = line_at(text, reached).1;
                }
                window *= 2;
                continue;
            }
            window = FIRST_WINDOW;
            let Some((kind, after, line_end)) = boundary.tag else {
                return;
            };
            let unit = Unit {
                start: boundary.end as u32,
                len: (line_end - boundary.end) as u32,
                kind,
                ctx,
                children: Vec::new(),
            };
            if emit(unit).is_break() {
                return;
            }
            ctx = after;
            from = line_at(text, boundary.end).1;
        }
    }
}

/// Parses `text` on `threads` threads, cut where nothing crosses: after a
/// blank line, outside containers, fences and `$$`, before a line that cannot
/// continue a list. The sequential scan for the cuts is part of the cost.
pub fn parse_threaded<P: BlockParser>(
    text: &str,
    threads: usize,
    make: impl Fn() -> P + Sync,
) -> Vec<Unit> {
    let cuts = safe_cuts(text, threads);
    let parts: Vec<Vec<Unit>> = std::thread::scope(|scope| {
        let make = &make;
        let handles: Vec<_> = cuts
            .windows(2)
            .map(|cut| {
                let (from, to) = (cut[0], cut[1]);
                scope.spawn(move || {
                    let mut units = make().parse(&text[from..to]);
                    units.iter_mut().for_each(|u| u.start += from as u32);
                    units
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    parts.concat()
}

fn safe_cuts(text: &str, parts: usize) -> Vec<usize> {
    let mut cuts = vec![0];
    let step = text.len() / parts + 1;
    let (mut pos, mut ctx, mut fence, mut blank_before) = (0, Ctx::Top, None, false);
    let mut math_until = 0;
    while pos < text.len() {
        let (line, next) = line_at(text, pos);
        let outside = fence.is_none() && pos >= math_until;
        if pos >= cuts.len() * step && blank_before && outside && ctx == Ctx::Top {
            let first = line.bytes().next();
            if first.is_some_and(|b| b.is_ascii_alphabetic() || b == b'#') {
                cuts.push(pos);
            }
        }
        match tag_line(line, ctx) {
            Some((_, after)) if outside || is_closer(line, ctx) => {
                ctx = after;
                fence = None;
            }
            _ if pos >= math_until => {
                let indent = line.len() - line.trim_start_matches(' ').len();
                if indent <= 3 {
                    fence = jade_core::fence::step(fence, &line[indent..]).1;
                }
                if fence.is_none() && jade_core::math_scan::opens_block(line) {
                    math_until = crate::own::math_block_end(text, pos).unwrap_or(0);
                }
            }
            _ => {}
        }
        blank_before = is_blank(line);
        pos = next;
    }
    cuts.push(text.len());
    cuts
}
