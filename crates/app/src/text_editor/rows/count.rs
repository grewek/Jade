//! Row counts: how many wrapped rows each line takes, without shaping it. The
//! pure, thread-split counters used by a cold rebuild, and the per-line
//! computations (with a `Window`) used to patch or verify single lines.

use crate::text_editor::preview::{LineIndent, is_thematic_break};
use crate::text_editor::table;
use gpui_kit::*;

use super::Env;
use crate::text_editor::columns::{self, Columns};
use crate::text_editor::doc::{DocumentCache, MathLines};
use crate::text_editor::element::region;
use crate::text_editor::image::{self, Sizes as ImageSizes};
use crate::text_editor::layout::{
    LineState, RowFontMetrics, build_display_lines, callout_at_line, char_budget, fit_with_indent,
    indent_px, shape_display_line, shaped_height, tall_rows, wrapped_row_count,
};
use crate::text_editor::lines::Fenwick;
use crate::text_editor::math::{self, Macros};
use crate::text_editor::preview::LineRole;
use crate::text_editor::preview::callout::Callout;
use crate::text_editor::preview::{
    DisplayLine, InlineStyle, LineContext, heading_prefix, line_indent,
};
use crate::text_editor::table::Table;
use jade_core::math_scan;

/// The parts of a [`DocumentCache`] a row count reads, as plain slices (the
/// cache itself holds `Rc`s, so it cannot cross to worker threads).
pub(super) struct DocView<'a> {
    pub(super) roles: &'a [LineRole],
    pub(super) contexts: &'a [LineContext],
    pub(super) tables: &'a [Table],
    pub(super) callouts: &'a [Callout],
    pub(super) images: &'a ImageSizes,
    pub(super) columns: &'a [Columns],
    pub(super) math: &'a [MathLines],
    pub(super) macros: &'a Macros,
    /// The region shown as source, laid out like plain text.
    pub(super) raw_region: Option<usize>,
    /// Set when regions are laid out side by side (see `DocumentCache::side_by_side`).
    pub(super) column_width: Option<f32>,
    /// Where each line starts, for finding the bytes of a stretch of lines.
    pub(super) line_index: &'a Fenwick,
}

/// What a row count depends on besides the document.
pub(super) struct RowParams {
    /// The table the cursor is in while focused: it renders as raw text.
    pub(super) active_table: Option<usize>,
    pub(super) available_width: Pixels,
    pub(super) metrics: RowFontMetrics,
    /// An image block's height is a whole number of these.
    pub(super) line_height: Pixels,
    /// For the rows a tall formula adds: the editor font size.
    pub(super) font_size: Pixels,
}

/// The index of the table [`is_table_active`] would call active, found once so
/// workers need not ask per line.
pub(super) fn active_table(doc: &DocumentCache, cursor: usize, focused: bool) -> Option<usize> {
    if !focused {
        return None;
    }
    let tables = doc.tables();
    let idx = tables.partition_point(|t| doc.table_byte_range(t).end < cursor);
    let table = tables.get(idx)?;
    (doc.table_byte_range(table).start <= cursor).then_some(idx)
}

/// Below this many lines a thread per core costs more than it saves.
const PARALLEL_MIN_LINES: usize = 20_000;

/// Lines a thread takes at a time: small enough that a stretch of costly lines
/// (a run of formulas) is shared out, big enough that fetching one is free.
const CHUNK_LINES: usize = 4096;

/// Most threads a rebuild uses.
const MAX_THREADS: usize = 16;

/// Every line's row count that needs no `Window`, split over threads on big
/// notes, plus the lines that do need one (their entry is a placeholder `0`).
pub(super) fn pure_row_counts(
    text: &str,
    view: &DocView,
    rows: &RowParams,
) -> (Vec<i64>, Vec<usize>) {
    let line_count = view.roles.len();
    let threads = if line_count < PARALLEL_MIN_LINES {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS))
    };
    // Threads fill their chunks of one array, so nothing is copied together
    // afterwards; lines with no rows keep the 0 they start as.
    let mut counts = vec![0i64; line_count];
    if threads == 1 {
        let shaped = pure_row_counts_in(text, view, rows, 0, &mut counts);
        return (counts, shaped);
    }
    let mut jobs: Vec<(usize, &mut [i64])> = counts.chunks_mut(CHUNK_LINES).enumerate().collect();
    jobs.reverse();
    let queue = std::sync::Mutex::new(jobs);
    let mut shaped: Vec<(usize, Vec<usize>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let job = queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
                        let Some((k, chunk)) = job else { break };
                        done.push((
                            k,
                            pure_row_counts_in(text, view, rows, k * CHUNK_LINES, chunk),
                        ));
                    }
                    done
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    shaped.sort_unstable_by_key(|&(k, _)| k);
    let shaped = shaped.into_iter().flat_map(|(_, lines)| lines).collect();
    (counts, shaped)
}

/// [`pure_row_counts`] for the lines from `first` on, as many as `counts` has
/// (zeroed): fills their counts and returns the lines that need a window.
fn pure_row_counts_in(
    text: &str,
    view: &DocView,
    rows: &RowParams,
    first: usize,
    counts: &mut [i64],
) -> Vec<usize> {
    let range = first..first + counts.len();
    let RowParams {
        active_table,
        available_width,
        metrics,
        line_height,
        font_size,
    } = *rows;
    let char_width = metrics.char_width;
    let ascent_descent = (metrics.ascent, metrics.descent);
    let line_count = view.roles.len();
    let mut shaped = Vec::new();
    // One sequential pass: `tables` are sorted, so table membership comes from
    // a cursor instead of a binary search per line. What is the same for every
    // row of a table (column widths) is worked out once per table.
    let budget = char_budget(available_width, char_width);
    let one_level_fit =
        fit_with_indent(available_width, char_width, LineIndent::Blockquote(1)).min(
            fit_with_indent(available_width, char_width, LineIndent::List(1)),
        );
    let mut next_table = view.tables.partition_point(|t| t.end_line <= range.start);
    let mut next_callout = view.callouts.partition_point(|c| c.end_line <= range.start);
    let mut next_region = view.columns.partition_point(|r| r.end_line <= range.start);
    let mut next_math = view.math.partition_point(|b| b.last_line < range.start);
    let mut table_state: Option<(usize, Option<Vec<usize>>)> = None;
    let starts = chunk_starts(text, view.line_index, first, counts.len());
    let dollar_lines = lines_with_dollar(text, &starts, first);
    let mut next_dollar = 0;
    for i in range {
        while dollar_lines.get(next_dollar).is_some_and(|&l| l < i) {
            next_dollar += 1;
        }
        let has_dollar = dollar_lines.get(next_dollar) == Some(&i);
        // A side-by-side region is one block: its first line holds the height
        // of its tallest column (which needs shaping, so it goes to `shaped`)
        // and the rest have no rows.
        if let Some(width) = view.column_width {
            while view
                .columns
                .get(next_region)
                .is_some_and(|r| r.end_line <= i)
            {
                next_region += 1;
            }
            let region = view.columns.get(next_region).filter(|r| r.start_line <= i);
            let shown = region.filter(|_| Some(next_region) != view.raw_region);
            if let Some(r) = shown.filter(|r| match r.kind {
                columns::Kind::Aside => columns::aside_split(width, r.floats()).is_some(),
                columns::Kind::Columns => columns::split(width, &r.percent[..r.count]).is_some(),
            }) {
                if i == r.start_line {
                    shaped.push(i);
                }
                continue;
            }
        }
        // A `$$` block's lines depend on its formula and on the caret: each is
        // counted with a window, the head first.
        while view.math.get(next_math).is_some_and(|b| b.last_line < i) {
            next_math += 1;
        }
        if view.math.get(next_math).is_some_and(|b| b.first_line <= i) {
            shaped.push(i);
            continue;
        }
        while view
            .callouts
            .get(next_callout)
            .is_some_and(|c| c.end_line <= i)
        {
            next_callout += 1;
        }
        if view.callouts.get(next_callout).is_some_and(|c| c.hides(i)) {
            continue;
        }
        let end = if i + 1 < line_count {
            starts[i - first + 1] - 1
        } else {
            starts[i - first + 1]
        };
        let line = &text[starts[i - first]..end];
        while view.tables.get(next_table).is_some_and(|t| t.end_line <= i) {
            next_table += 1;
        }
        let table = view.tables.get(next_table).filter(|t| t.start_line <= i);
        let count = match table {
            None => estimate_row_count(
                line,
                view,
                i,
                (char_width, line_height),
                available_width,
                (one_level_fit, has_dollar, font_size, ascent_descent),
            ),
            Some(table) => {
                let is_header = callout_at_line(view.callouts, i).is_some_and(|c| c.is_header(i));
                let plain = matches!(view.roles[i], LineRole::Plain) && !is_header;
                if table_state.as_ref().is_none_or(|(n, ..)| *n != next_table) {
                    // A table that fits its width has no wrapped cell in any row,
                    // so its widths (and its rows' text) are never looked at.
                    let widths = if table::fits_unshrunk(&table.columns, budget) {
                        None
                    } else {
                        Some(table::resolve_widths(&table.columns, budget))
                    };
                    table_state = Some((next_table, widths));
                }
                match &table_state {
                    // An inactive, plainly rendered table: a delimiter has no row
                    // (#48), a row is as tall as its tallest wrapped cell.
                    // A row with a `$` may draw a tall formula, which only
                    // shaping measures.
                    Some((n, widths)) if plain && active_table != Some(*n) && !has_dollar => {
                        match widths {
                            _ if table.is_delimiter(i) => 0,
                            Some(widths) => table::row_line_count(line, widths),
                            None => 1,
                        }
                    }
                    _ => {
                        shaped.push(i);
                        0
                    }
                }
            }
        };
        counts[i - first] = count as i64;
    }
    shaped
}

/// Where each of the `len` lines from line `first` starts, and the start of the
/// line after them (the text's length at the end): found from the first line's
/// offset by scanning for newlines, so a rebuild keeps no per-line table.
fn chunk_starts(text: &str, index: &Fenwick, first: usize, len: usize) -> Vec<usize> {
    let mut at = index.prefix_sum(first) as usize;
    let mut starts = Vec::with_capacity(len + 1);
    starts.push(at);
    for _ in 0..len {
        at = text[at..].find('\n').map_or(text.len(), |p| at + p + 1);
        starts.push(at);
    }
    starts
}

/// The lines (numbered from `first`, whose starts are `starts`) that hold a
/// `$`, found in one pass over their bytes: a check on every line cost more
/// than the rest of the estimate on a big note.
fn lines_with_dollar(text: &str, starts: &[usize], first: usize) -> Vec<usize> {
    let (base, end) = (starts[0], starts[starts.len() - 1].min(text.len()));
    let hay = &text[base..end];
    let mut lines = Vec::new();
    let mut at = 0;
    while let Some(found) = hay[at..].find('$') {
        let line = starts.partition_point(|&s| s <= base + at + found) - 1;
        lines.push(first + line);
        at = starts[line + 1].min(end) - base;
    }
    lines
}

/// A cheap, allocation-free over-estimate of `line`'s (line `i`'s) row count
/// — the cold-`rebuild` fast path for an *ordinary* line; the caller must
/// not pass a table row or delimiter, which get the exact computation
/// instead (see [`RowHeightCache::rebuild`]'s own doc comment for why). Uses the line's
/// *raw* length rather than running `build_display_lines`'s real Live-
/// Preview inline scan at all: always safe (an over-estimate, never
/// fewer rows than the truth) because every hiding/replacement transform
/// this editor's Live Preview does — markup removal, a bullet/checkbox
/// glyph, a callout's icon + capitalized type name — only ever shrinks
/// what's displayed relative to the raw source, never grows it. Also
/// exact, not just a safe over-estimate, for the active line and for
/// fenced code content, both of which always render their raw text
/// verbatim anyway.
///
/// Still needs the indent level right (a wider wrap width than the real
/// one would *under*-estimate, the one way this could go unsafe) — reuses
/// `preview::line_indent` (the same cheap prefix-only check `build_
/// display_line` itself uses, not the expensive scan) combined with the
/// line's already-known `LineContext` (from `DocumentCache`, itself
/// already computed — not rederived here) the same way `build_display_
/// line_in_context` resolves it for real, except for fenced code content,
/// whose indent is always forced to `None` regardless (see
/// `build_display_lines`'s own fence-content override).
fn estimate_row_count(
    line: &str,
    doc: &DocView,
    i: usize,
    (char_width, line_height): (Pixels, Pixels),
    available_width: Pixels,
    (one_level_fit, has_dollar, font_size, text): (usize, bool, Pixels, (f32, f32)),
) -> usize {
    // A block's height does not depend on the width or the line's length, and is
    // more than the one row the fast path below would give it.
    if image::may_be_image(line)
        && matches!(doc.roles[i], LineRole::Plain)
        && matches!(doc.contexts[i], LineContext::Plain)
        && callout_at_line(doc.callouts, i).is_none()
        && let Some(embed) = image::embed_line(line)
    {
        return image::rows(line, &embed, Some(doc.images), available_width, line_height);
    }
    // A plain `#` heading: wider characters, several body rows per row (#58).
    // One in a quote or list item is left to the in-view verify.
    if !has_dollar
        && line.as_bytes().first() == Some(&b'#')
        && matches!(doc.roles[i], LineRole::Plain)
        && matches!(doc.contexts[i], LineContext::Plain)
        && let Some((level, prefix)) = heading_prefix(line)
    {
        return heading_row_estimate(&line[prefix..], level, available_width, char_width);
    }
    // A line with no context of its own that doesn't start with `>` is
    // indented at most one level (a list marker; blockquotes need a `>`), so
    // if even that narrower width holds its bytes it is a single row — most
    // lines, decided without looking at its indent at all.
    if matches!(doc.contexts[i], LineContext::Plain)
        && !matches!(doc.roles[i], LineRole::FenceContent)
        && line.as_bytes().first() != Some(&b'>')
        && !has_dollar
        && line.len() <= one_level_fit
    {
        return 1;
    }
    let is_code = matches!(doc.roles[i], LineRole::FenceContent);
    // Fenced code is inset into its box and set at its own size.
    let char_width = if is_code {
        char_width * crate::settings::code_font_scale()
    } else {
        char_width
    };
    let indent = if is_code {
        if crate::settings::code_inset() {
            LineIndent::Code
        } else {
            LineIndent::None
        }
    } else {
        match doc.contexts[i] {
            LineContext::Plain => line_indent(line),
            LineContext::ListMarkerLine(depth) => LineIndent::List(depth),
            LineContext::LazyContinuation(indent) => indent,
        }
    };
    let wrap_width = Some((available_width - indent_px(indent)).max(px(1.)));
    // The empty rows `list.item_gap` leaves below a list item.
    let gap = match doc.contexts[i] {
        LineContext::ListMarkerLine(_) if !is_thematic_break(line) => {
            crate::settings::list_item_gap()
        }
        _ => 0,
    };
    // A formula is a few blank columns that may be taller than a row (#20): the
    // rows are counted on what is left of the text, each as many as it needs.
    if has_dollar
        && matches!(doc.roles[i], LineRole::Plain)
        && let Some((mut formulas, chars)) = inline_math_extent(
            line,
            doc.macros,
            fit_with_indent(available_width, char_width, indent),
        )
    {
        let per_row = chars_per_row(wrap_width, char_width);
        let moved = settle_formulas(&mut formulas, per_row);
        let rows = rows_for_chars(chars + moved, wrap_width, char_width);
        return rows + gap + tall_extra_rows(&formulas, per_row, (font_size, line_height), text);
    }
    wrapped_row_count(line, wrap_width, char_width).max(1) + gap
}

/// The rows of a heading's `text` (its `#` prefix hidden) at `level`: the rows
/// its wider characters wrap to, each `settings::heading_rows` body rows tall,
/// plus the empty rows around it.
fn heading_row_estimate(
    text: &str,
    level: u8,
    available_width: Pixels,
    char_width: Pixels,
) -> usize {
    let scale = crate::settings::heading_scale(level);
    let units = crate::settings::heading_rows(scale);
    let wide = char_width * scale;
    let (above, below) = crate::settings::heading_space_rows(level);
    rows_for_chars(text.chars().count(), Some(available_width), wide) * units + above + below
}

/// A drawn formula as the row estimate sees it: the display character it starts
/// at, how many blank columns it takes, and how far it reaches above and below
/// the baseline (em).
#[derive(Clone, Copy)]
pub(super) struct FormulaSpot {
    start: usize,
    columns: usize,
    reach: (f32, f32),
}

/// Moves each formula that would run past the end of its row to the start of the
/// next, as the wrap does (a formula's blank columns are one unbreakable word), and
/// returns how many characters of room that leaves empty. `start`s are in
/// characters; a formula wider than a row stays where it is.
fn settle_formulas(formulas: &mut [FormulaSpot], per_row: usize) -> usize {
    let mut moved = 0;
    for f in formulas {
        f.start += moved;
        let column = f.start % per_row;
        if column + f.columns > per_row && f.columns <= per_row {
            let gap = per_row - column;
            f.start += gap;
            moved += gap;
        }
    }
    moved
}

/// How many rows more than its text a line needs for its tall formulas: each
/// row is as tall as its tallest formula (#32). The row a formula is on is
/// guessed from its character offset (monospace, no word wrapping), so a formula
/// the wrap moves to the next row can be off; painting corrects the count. A
/// formula that would be split by the row's end is not drawn and counts for
/// nothing, as when shaped. `per_row` is the characters a row holds.
fn tall_extra_rows(
    formulas: &[FormulaSpot],
    per_row: usize,
    (font_size, line_height): (Pixels, Pixels),
    text: (f32, f32),
) -> usize {
    let (mut extra, mut row, mut reach) = (0, 0, (0.0f32, 0.0f32));
    let mut flush = |reach: (f32, f32)| {
        extra += tall_rows(reach, (font_size, line_height), text) - 1;
    };
    for f in formulas {
        let (first, last) = (
            f.start / per_row,
            (f.start + f.columns.max(1) - 1) / per_row,
        );
        if first != last {
            continue;
        }
        if first != row {
            flush(reach);
            (row, reach) = (first, (0.0, 0.0));
        }
        reach = (reach.0.max(f.reach.0), reach.1.max(f.reach.1));
    }
    flush(reach);
    extra
}

/// The drawn formulas of a display line, for [`tall_extra_rows`]; none (and no
/// allocation) for a line without a drawn formula.
fn formula_spots(dl: &DisplayLine, line_text: &str, macros: &Macros) -> Vec<FormulaSpot> {
    dl.segments
        .iter()
        .filter(|s| s.style == InlineStyle::Math)
        .filter_map(|seg| {
            let drawn = math::rendered_span(line_text.get(seg.raw.clone())?, macros)?;
            Some(FormulaSpot {
                start: dl.display_text[..seg.display.start].chars().count(),
                columns: dl.display_text[seg.display.clone()].chars().count(),
                reach: (drawn.layout.ascent, drawn.layout.descent),
            })
        })
        .collect()
}

/// The characters a row of `wrap_width` holds.
fn chars_per_row(wrap_width: Option<Pixels>, char_width: Pixels) -> usize {
    wrap_width
        .filter(|_| char_width > px(0.))
        .map_or(usize::MAX, |w| ((w / char_width) as usize).max(1))
}

/// The drawn inline formulas of `line`, and how many display characters the line
/// has with each formula as its blank columns; `None` when it draws none. A
/// formula wider than `fit` columns (a row; 0 = unknown) is shown as its source,
/// as the shaped line does (#49), so it counts as plain characters.
fn inline_math_extent(
    line: &str,
    macros: &Macros,
    fit: usize,
) -> Option<(Vec<FormulaSpot>, usize)> {
    let (mut formulas, mut chars, mut last) = (Vec::new(), 0, 0);
    let mut nest = Nesting::default();
    math_scan::scan_inline(line, 0, line.len(), |found| {
        nest.advance(line.as_bytes(), found.span.start);
        // Emphasis, strikethrough and link text keep a formula as written:
        // it is counted as the plain characters it is, and makes no row tall.
        if nest.inside() {
            return;
        }
        if let Some(drawn) = math::rendered_span(&line[found.span.clone()], macros)
            && (fit == 0 || drawn.columns <= fit)
        {
            chars += line[last..found.span.start].chars().count();
            formulas.push(FormulaSpot {
                start: chars,
                columns: drawn.columns,
                reach: (drawn.layout.ascent, drawn.layout.descent),
            });
            chars += drawn.columns;
            last = found.span.end;
        }
    });
    (last > 0).then(|| (formulas, chars + line[last..].chars().count()))
}

/// Whether a position of a line is inside an emphasis (`*`, `**`), a
/// strikethrough (`~~`) or a bracket pair, as far as a cheap left-to-right
/// pass can tell: a delimiter run opens when a word follows it and closes when
/// one precedes it, so a list bullet or a lone `*` opens nothing.
#[derive(Default)]
struct Nesting {
    at: usize,
    star: bool,
    tilde: bool,
    brackets: u32,
}

impl Nesting {
    /// Moves over `bytes[self.at..to]`.
    fn advance(&mut self, bytes: &[u8], to: usize) {
        let mut k = self.at;
        while k < to {
            let b = bytes[k];
            match b {
                b'[' => self.brackets += 1,
                b']' => self.brackets = self.brackets.saturating_sub(1),
                b'*' | b'~' => {
                    let run = bytes[k..].iter().take_while(|&&c| c == b).count();
                    let before = k.checked_sub(1).map(|p| bytes[p]);
                    let after = bytes.get(k + run).copied();
                    let word = |c: Option<u8>| c.is_some_and(|c| !c.is_ascii_whitespace());
                    let flag = if b == b'*' {
                        &mut self.star
                    } else {
                        &mut self.tilde
                    };
                    let delimiter = b == b'*' || run >= 2;
                    if delimiter && ((!*flag && word(after)) || (*flag && word(before))) {
                        *flag = !*flag;
                    }
                    k += run;
                    continue;
                }
                _ => {}
            }
            k += 1;
        }
        self.at = to.max(self.at);
    }

    fn inside(&self) -> bool {
        self.star || self.tilde || self.brackets > 0
    }
}

/// [`wrapped_row_count`] for text of `chars` characters.
fn rows_for_chars(chars: usize, wrap_width: Option<Pixels>, char_width: Pixels) -> usize {
    let Some(wrap_width) = wrap_width.filter(|_| char_width > px(0.)) else {
        return 1;
    };
    let per_row = ((wrap_width / char_width) as usize).max(1);
    chars.max(1).div_ceil(per_row)
}

/// The line whose row count line `i` is part of: the first line of its
/// side-by-side region, else itself.
pub(super) fn row_owner(doc: &DocumentCache, i: usize) -> usize {
    doc.side_by_side(i)
        .map_or(i, |(region, _)| region.start_line)
}

/// The row count for line `i` alone, as the cold rebuild would count it. The
/// first line of a side-by-side region gets the region's height and its other
/// lines none.
pub(super) fn row_count_for_line(env: &Env, i: usize, window: &mut Window) -> usize {
    if let Some((region, split)) = env.doc.side_by_side(i) {
        if i != region.start_line {
            return 0;
        }
        if region.is_aside() {
            return aside_row_estimate(env, (&region, &split), window);
        }
        let widths = (0..region.count).map(|k| px(split.width(k)));
        let mut height = 1;
        for (k, width) in widths.enumerate() {
            let rows: usize = region
                .column_lines(k)
                .map(|line| {
                    if env.doc.math_block_at(line).is_some() {
                        measure_math_block(
                            env,
                            line,
                            crate::text_editor::settings::body_row_height(),
                        );
                    }
                    let state = LineState::in_column(env.doc, line, env.cursor, env.focused);
                    state_row_count(env, line, (&state, width), window)
                })
                .sum();
            height = height.max(rows);
        }
        return height;
    }
    measure_math_block(env, i, crate::text_editor::settings::body_row_height());
    // A `$$` block shown as a formula is its first line's `rows` and nothing on
    // the others: no need to build and wrap them.
    if let Some(block) = env
        .doc
        .math_block_at(i)
        .filter(|b| !env.doc.math_block_active(b, env.cursor, env.focused))
        && let Some(rows) = block.rows()
    {
        return if block.first_line == i { rows } else { 0 };
    }
    let state = LineState::of(env.doc, i, env.cursor, env.focused);
    state_row_count(env, i, (&state, env.width), window)
}

/// If line `i` opens a `$$` block, sets how many rows its formula takes (none
/// if it does not parse), before any line of the block is shaped.
pub(super) fn measure_math_block(env: &Env, i: usize, line_height: Pixels) {
    let Some(block) = env.doc.math_block_at(i).filter(|b| b.first_line == i) else {
        return;
    };
    let body = env.doc.math_body(env.text, block);
    let rows = math::rendered(body, true, env.doc.macros()).map_or(0, |r| {
        math::block_rows(
            (r.layout.ascent, r.layout.descent),
            f32::from(env.font_size),
            f32::from(line_height),
        )
    });
    block.set_rows(rows);
}

/// An aside's rows without shaping it: the taller float stack, or that plus
/// what the text needs past the stacks (it wraps at the narrow channel beside
/// them and at the full width below). Summing every column's rows, as a
/// side-by-side region does, counts the text as if it stayed in the channel,
/// far too tall for a float block, and the scrollbar shifted as paint
/// corrected it.
fn aside_row_estimate(
    env: &Env,
    (region, split): (&Columns, &columns::Split),
    window: &mut Window,
) -> usize {
    let rows_of = |lines: &mut dyn Iterator<Item = usize>, width: Pixels, window: &mut Window| {
        lines
            .map(|line| {
                if env.doc.math_block_at(line).is_some() {
                    measure_math_block(env, line, crate::text_editor::settings::body_row_height());
                }
                let state = LineState::in_column(env.doc, line, env.cursor, env.focused);
                state_row_count(env, line, (&state, width), window)
            })
            .sum::<usize>()
    };
    let text_part = region.count - 1;
    let mut stacks = [0usize; 2];
    for (k, spec) in region.floats().iter().enumerate() {
        let rows = rows_of(&mut region.column_lines(k), px(split.width(k)), window);
        // A row of air between floats on one side.
        stacks[usize::from(spec.right)] += rows + 1;
    }
    let stack = stacks[0].max(stacks[1]);
    let narrow = split.width(text_part);
    let channel_rows = rows_of(&mut region.text_lines(), px(narrow), window);
    let beyond = channel_rows.saturating_sub(stack);
    let full = split.total.max(narrow);
    let below = (beyond as f32 * narrow / full).ceil() as usize;
    (stack + below).max(1)
}

/// The count for a line a patch recomputes. A patch leaves the line in the
/// verified window, so an estimate put there would stay until paint corrects
/// it, after anything positioned from the rows (scrolling to the caret) has
/// used it. So with `exact` the line is shaped, as painting will see it
/// (a region as a whole); without, it is the monospace estimate, for edits
/// that touch too many lines to shape.
pub(super) fn patched_row_count(env: &Env, i: usize, exact: bool, window: &mut Window) -> usize {
    if exact && env.doc.side_by_side(i).is_none() {
        return shaped_row_count_for_line(env, i, window);
    }
    estimated_patch_count(env, i, window)
}

/// A region's shaped height, or the estimate of any other line: what a patch
/// counts for lines it does not shape.
fn estimated_patch_count(env: &Env, i: usize, window: &mut Window) -> usize {
    if let Some((region, split)) = env.doc.side_by_side(i)
        && i == region.start_line
    {
        return region::shape_region(
            env.text,
            env.doc,
            (&region, &split),
            (env.cursor, env.focused),
            (
                env.font,
                env.font_size,
                crate::text_editor::settings::body_row_height(),
            ),
            (env.text_color.into(), None),
            window,
        )
        .rows;
    }
    row_count_for_line(env, i, window)
}

/// Line `i`'s rows when shown as `state` says, wrapped at `width`; a tall
/// formula grows them, in a column as anywhere (#23).
fn state_row_count(
    env: &Env,
    i: usize,
    (state, width): (&LineState, Pixels),
    window: &mut Window,
) -> usize {
    let line_text = &env.text[env.doc.line_start_byte(i)..env.doc.line_end_byte(i)];
    let (display_lines, wrap_width) = build_display_lines(
        line_text,
        state,
        env.font,
        env.font_size,
        env.text_color,
        width,
        window,
    );
    let line_height = crate::text_editor::settings::body_row_height();
    let tall = |dl: &DisplayLine| {
        let mut formulas = formula_spots(dl, line_text, env.doc.macros());
        // A heading's formulas are drawn at its size, in its rows of `units` body rows.
        let (scale, units) = (dl.font_scale(), dl.row_units());
        let text = (env.metrics.ascent * scale, env.metrics.descent * scale);
        let per_row = chars_per_row(wrap_width, env.metrics.char_width * scale);
        let moved = settle_formulas(&mut formulas, per_row);
        let chars = dl.display_text.chars().count();
        // The room left empty before a moved formula can cost a row of text.
        let pushed = ((chars + moved).div_ceil(per_row) - chars.div_ceil(per_row)) * units;
        let metrics = (env.font_size * scale, line_height * units as f32);
        pushed + tall_extra_rows(&formulas, per_row, metrics, text) * units
    };
    display_lines
        .iter()
        .map(|dl| {
            // A heading's characters are wider and each of its rows is several
            // body rows (#58); its formulas are not yet sized with it (C6).
            let char_width = env.metrics.char_width * dl.font_scale();
            let (above, below) = dl.space_rows();
            wrapped_row_count(&dl.display_text, wrap_width, char_width) * dl.row_units()
                + above
                + below
                + tall(dl)
        })
        .sum::<usize>()
        .max(usize::from(!state.hidden))
}

/// The row count painting will see for line `i`: real shaping, so word
/// wrapping is included. [`row_count_for_line`]'s monospace estimate can come
/// out one row short of that, which verifying must not inherit.
pub(super) fn shaped_row_count_for_line(env: &Env, i: usize, window: &mut Window) -> usize {
    let line_height = crate::text_editor::settings::body_row_height();
    measure_math_block(env, i, line_height);
    let state = LineState::of(env.doc, i, env.cursor, env.focused);
    let shaped = shape_display_line(
        &env.text[env.doc.line_start_byte(i)..env.doc.line_end_byte(i)],
        &state,
        env.font,
        env.font_size,
        env.text_color,
        env.width,
        window,
    );
    let text = &env.text[env.doc.line_start_byte(i)..env.doc.line_end_byte(i)];
    let (_, rows) = shaped_height(
        &shaped,
        text,
        (env.font_size, line_height),
        env.doc.macros(),
    );
    rows.max(usize::from(!state.hidden))
}
