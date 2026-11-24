//! A tall formula makes its line taller wherever it is drawn, the cold row
//! estimate agrees with shaping there, and a region's height follows its
//! lines (#23). Where a formula is not drawn (a table cell shows its source)
//! the line stays one row.

use super::super::Editor;
use super::region::shape_region;
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::layout::{EntryHeight, LineState, entry_heights, shape_display_line};
use crate::text_editor::rows::{self, RowCache};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Pixels, TestAppContext, WindowOptions, px};

const TALL: &str = "$\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}$";
const WIDTH: f32 = 800.;

/// `(context, note text, the line holding the formula, whether it is drawn)`.
fn contexts() -> Vec<(&'static str, String, usize, bool)> {
    let at = |name: &'static str, body: &str, line: usize, drawn: bool| {
        (name, format!("intro\n\n{body}\n\nlast\n"), line, drawn)
    };
    vec![
        at("paragraph", &format!("text {TALL} here"), 2, true),
        at("heading", &format!("## head {TALL} here"), 2, true),
        at("bullet", &format!("- item {TALL} here"), 2, true),
        at(
            "nested bullet",
            &format!("- a\n  - item {TALL} here"),
            3,
            true,
        ),
        at("numbered", &format!("1. item {TALL} here"), 2, true),
        at("task", &format!("- [ ] item {TALL} here"), 2, true),
        at("quote", &format!("> text {TALL} here"), 2, true),
        at("nested quote", &format!("> > text {TALL} here"), 2, true),
        at(
            "callout body",
            &format!("> [!note]\n> text {TALL} here"),
            3,
            true,
        ),
        at(
            "callout header",
            &format!("> [!note] title {TALL}\n> body"),
            2,
            true,
        ),
        at(
            "table cell",
            &format!("| a | {TALL} |\n| - | - |\n| c | d |"),
            2,
            true,
        ),
        at(
            "table cell, small formula",
            "| a | $x^2$ |\n| - | - |\n| c | d |",
            2,
            false,
        ),
    ]
}

fn open(cx: &mut TestAppContext) -> gpui_kit::AnyWindowHandle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new("", cx))
        })
        .unwrap()
        .0
    })
}

#[gpui_kit::test]
fn a_tall_formula_grows_its_line_wherever_it_is_drawn(cx: &mut TestAppContext) {
    let window = open(cx);
    for (name, text, line, drawn) in contexts() {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let look = (&font, size, style.color);
            let doc = DocumentCache::new(&text);
            let mut cache = RowCache::empty();
            rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
            let cold = rows::count_at(&cache, line);
            let shaped =
                rows::shaped_count_for_test(&text, &doc, line, (0, false), px(WIDTH), look, window);
            assert_eq!(drawn, shaped >= 2, "{name}: {shaped} rows when shaped");
            assert_eq!(
                cold, shaped as i64,
                "{name}: cold estimate {cold}, shaped {shaped}"
            );
        })
        .unwrap();
    }
}

/// A formula wider than a row is shown as its source and wraps as text (#49):
/// the cold estimate is at most a text wrap short of shaping, and no row is made
/// tall for it.
#[gpui_kit::test]
fn a_formula_wider_than_a_row_is_source_and_agrees_with_shaping(cx: &mut TestAppContext) {
    let window = open(cx);
    let terms = (1..=60).map(|k| format!("x_{{{k}}}")).collect::<Vec<_>>();
    let wide = format!(
        "$\\begin{{pmatrix}} {} \\end{{pmatrix}}$",
        terms.join(" & ")
    );
    for (name, body) in [
        ("paragraph", format!("text {wide} here")),
        ("heading", format!("## head {wide} here")),
        ("bullet", format!("- item {wide} here")),
        ("quote", format!("> text {wide} here")),
    ] {
        let text = format!("intro\n\n{body}\n\nlast\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let look = (&font, size, style.color);
            let doc = DocumentCache::new(&text);
            let mut cache = RowCache::empty();
            rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
            let cold = rows::count_at(&cache, 2);
            let shaped =
                rows::shaped_count_for_test(&text, &doc, 2, (0, false), px(WIDTH), look, window);
            // Text wraps at words, which the monospace estimate does not see: it
            // may be one row short, as for any text; painting makes it exact.
            assert!(
                (shaped as i64 - cold) == 0 || (shaped as i64 - cold) == 1,
                "{name}: cold {cold}, shaped {shaped}"
            );
            // The source wraps over several rows, one text row each, none taller.
            let chars = body.chars().count();
            let line_height = f32::from(window.line_height());
            let char_width = f32::from(size) * 0.6;
            let per_row = (WIDTH / char_width) as usize;
            assert!(
                shaped >= chars / per_row && shaped <= chars / per_row + 3,
                "{name}: {shaped} rows for {chars} characters ({per_row} a row, {line_height}px)"
            );
        })
        .unwrap();
    }
}

/// A region holding the formula in each of its kinds of column.
fn regions() -> Vec<(&'static str, String)> {
    let body = format!("text {TALL} here");
    vec![
        (
            "columns",
            format!("::: columns\n\n{body}\n\n---\n\nright\n\n:::\n"),
        ),
        (
            "columns quote",
            format!("::: columns\n\n> {body}\n\n---\n\nright\n\n:::\n"),
        ),
        (
            "aside text",
            format!("::: aside left 30%\n\nfloat\n\n---\n\n{body}\n\n:::\n"),
        ),
        (
            "aside float",
            format!("::: aside left 30%\n\n{body}\n\n---\n\nplain text\n\n:::\n"),
        ),
        (
            "aside quote",
            format!("::: aside left 30%\n\n> [!cite]\n> {body}\n\n---\n\nplain text\n\n:::\n"),
        ),
    ]
}

#[gpui_kit::test]
fn a_region_is_as_tall_as_the_lines_it_holds(cx: &mut TestAppContext) {
    let window = open(cx);
    for (name, text) in regions() {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let line_height = window.line_height();
            let look = (&font, size, style.color);
            let mut doc = DocumentCache::new(&text);
            doc.view.set_column_width(Some(WIDTH));
            let mut cache = RowCache::empty();
            rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
            let (region, split) = doc.side_by_side(0).expect(name);
            let shaped = shape_region(
                &text,
                &doc,
                (&region, &split),
                (usize::MAX, false),
                (&font, size, line_height),
                (style.color.into(), None),
                window,
            );
            // The formula's line is taller than a row, and no line of a column
            // starts above where the one before it ends.
            let tall = shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .any(|sl| sl.height >= line_height * 2.);
            assert!(tall, "{name}: no line grew");
            for column in &shaped.columns {
                for pair in column.lines.windows(2) {
                    assert!(
                        pair[1].y >= pair[0].y + pair[0].height - px(0.01),
                        "{name}: line {} overlaps line {}",
                        pair[1].line,
                        pair[0].line
                    );
                }
            }
            // The cache holds the region's shaped height, as the first frame
            // after a rebuild verifies.
            rows::verify_for_test(&mut cache, &text, &doc, 0..doc.line_count(), look, window);
            assert_eq!(
                rows::count_at(&cache, 0),
                shaped.rows as i64,
                "{name}: cached region height"
            );
            let _ = LineState::plain();
        })
        .unwrap();
    }
}

/// A formula that is tall only through a macro of the note grows its line, and
/// the cold estimate agrees with shaping (#14); changing the definition away
/// from a tall one shrinks it again.
#[gpui_kit::test]
fn a_tall_formula_made_by_a_macro_grows_its_line(cx: &mut TestAppContext) {
    let window = open(cx);
    let define = "$\\newcommand{\\M}{\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}}$";
    let tall = format!("{define}\n\ntext $\\M$ here\n\nlast\n");
    let flat = tall.replace("\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}", "x");
    for (name, text, rows_expected) in [("tall", tall, true), ("flat", flat, false)] {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let look = (&font, size, style.color);
            let doc = DocumentCache::new(&text);
            let mut cache = RowCache::empty();
            rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
            let cold = rows::count_at(&cache, 2);
            let shaped =
                rows::shaped_count_for_test(&text, &doc, 2, (0, false), px(WIDTH), look, window);
            assert_eq!(rows_expected, shaped >= 2, "{name}: {shaped} rows");
            assert_eq!(cold, shaped as i64, "{name}: cold {cold}, shaped {shaped}");
        })
        .unwrap();
    }
}

const BLOCK: &str = "$$\n\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}\n$$";

/// A `$$` block inside a column or an aside is drawn as a formula over the
/// rows its height needs (#18), and the region is as tall as its lines.
#[gpui_kit::test]
fn a_block_in_a_region_is_a_formula_over_its_rows(cx: &mut TestAppContext) {
    let window = open(cx);
    let regions = [
        (
            "columns",
            format!("::: columns\n\nleft\n\n{BLOCK}\n\n---\n\nright\n\n:::\n"),
        ),
        (
            "aside text",
            format!("::: aside left 30%\n\nfloat\n\n---\n\n{BLOCK}\n\nmore\n\n:::\n"),
        ),
        (
            "aside float",
            format!("::: aside left 30%\n\n{BLOCK}\n\n---\n\nplain text\n\n:::\n"),
        ),
    ];
    for (name, text) in regions {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let line_height = window.line_height();
            let look = (&font, size, style.color);
            let mut doc = DocumentCache::new(&text);
            doc.view.set_column_width(Some(WIDTH));
            let mut cache = RowCache::empty();
            rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
            let block = doc.math_blocks().first().expect(name);
            let rows = block
                .rows()
                .unwrap_or_else(|| panic!("{name}: not measured"));
            assert!(rows >= 2, "{name}: {rows} rows");
            let (first, last) = (block.first_line, block.last_line);
            let (region, split) = doc.side_by_side(0).expect(name);
            let shaped = shape_region(
                &text,
                &doc,
                (&region, &split),
                (usize::MAX, false),
                (&font, size, line_height),
                (style.color.into(), None),
                window,
            );
            let lines: Vec<_> = shaped.columns.iter().flat_map(|c| &c.lines).collect();
            let head = lines.iter().find(|sl| sl.line == first).expect(name);
            assert_eq!(head.height, line_height * rows as f32, "{name}: head");
            for sl in lines.iter().filter(|sl| sl.line > first && sl.line <= last) {
                assert_eq!(sl.height, px(0.), "{name}: line {} is hidden", sl.line);
            }
            rows::verify_for_test(&mut cache, &text, &doc, 0..doc.line_count(), look, window);
            assert_eq!(
                rows::count_at(&cache, 0),
                shaped.rows as i64,
                "{name}: cached region height"
            );
            // With the caret in it the block is its source lines.
            let inside = doc.line_start_byte(first + 1);
            let open = shape_region(
                &text,
                &doc,
                (&region, &split),
                (inside, true),
                (&font, size, line_height),
                (style.color.into(), None),
                window,
            );
            let lines: Vec<_> = open.columns.iter().flat_map(|c| &c.lines).collect();
            let source: Pixels = lines
                .iter()
                .filter(|sl| (first..=last).contains(&sl.line))
                .map(|sl| sl.height)
                .sum();
            assert!(source >= line_height * 3., "{name}: source rows");
        })
        .unwrap();
    }
}

/// A block that runs across a divider is not one formula: it stays source.
#[gpui_kit::test]
fn a_block_across_a_divider_stays_source(cx: &mut TestAppContext) {
    let window = open(cx);
    let text = "::: columns\n\nleft\n\n$$\nx\n\n---\n\ny\n$$\n\n:::\n";
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let mut doc = DocumentCache::new(text);
        doc.view.set_column_width(Some(WIDTH));
        let Some(block) = doc.math_blocks().first() else {
            return;
        };
        block.set_rows(3);
        let first = block.first_line;
        let state = LineState::in_column(&doc, first, usize::MAX, false);
        assert_eq!(state.math, crate::text_editor::layout::MathPart::None);
        assert!(!state.hidden);
        let _ = cx;
    })
    .unwrap();
}

/// A paragraph of plain words wrapped over several rows with a tall formula in
/// the second; where a formula is not drawn the same text takes one row less.
fn long_paragraph() -> String {
    let words = |n: usize| "word ".repeat(n);
    format!("{}{TALL} {}", words(22), words(120).trim_end())
}

/// Shapes `text` (one paragraph) at the test width and returns the entry's
/// shaped line, its display line and its row heights.
fn shaped_paragraph(
    text: &str,
    width: Pixels,
    window: &mut gpui_kit::Window,
) -> (
    gpui_kit::WrappedLine,
    crate::text_editor::preview::DisplayLine,
    EntryHeight,
) {
    let style = window.text_style();
    let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
    let state = LineState::plain();
    let mut results = shape_display_line(text, &state, &font, size, style.color, width, window);
    let doc = DocumentCache::new(text);
    let entry = entry_heights(&results, text, (size, window.line_height()), doc.macros())
        .next()
        .unwrap();
    let (wrapped, display) = results.remove(0);
    (wrapped, display, entry)
}

#[gpui_kit::test]
fn only_the_rows_holding_a_tall_formula_are_tall(cx: &mut TestAppContext) {
    let window = open(cx);
    let text = long_paragraph();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let (wrapped, _, entry) = shaped_paragraph(&text, px(WIDTH), window);
        let rows = wrapped.wrap_boundaries().len() + 1;
        assert!(
            rows >= 4,
            "the paragraph must wrap over several rows: {rows}"
        );
        assert_eq!(entry.row, window.line_height());
        assert_eq!(entry.varied.len(), rows, "rows differ in height");
        let tall = entry.varied.iter().filter(|&&u| u > 1).count();
        assert_eq!(tall, 1, "one row holds the formula: {:?}", entry.varied);
        let units: usize = entry.varied.iter().map(|&u| usize::from(u)).sum();
        assert_eq!(entry.height, window.line_height() * units as f32);
        assert!(
            units < rows * 2,
            "not every row grew: {units} units, {rows} rows"
        );
        // The rows tile the entry: each starts where the one before ends.
        for row in 1..rows {
            assert_eq!(
                entry.top(row),
                entry.top(row - 1) + entry.row_height(row - 1)
            );
        }
        // A position GPUI's layout gives lands on its row's top, with its height.
        for row in 0..rows {
            // One character in: the index of a row's first glyph is the end of the row before.
            let at = super::justify::row_start(&wrapped, row).unwrap() + 1;
            let layout = wrapped.position_for_index(at, entry.row).unwrap();
            let (placed, height) = entry.place(layout);
            assert_eq!(placed.y, entry.top(row), "row {row}");
            assert_eq!(height, entry.row_height(row), "row {row}");
            assert_eq!(placed.x, layout.x);
        }
        // A y inside a row finds that row, and the layout y is GPUI's for it.
        for row in 0..rows {
            let inside = entry.top(row) + entry.row_height(row) / 2.;
            assert_eq!(entry.row_at(inside), row);
            assert_eq!(entry.layout_y(inside), window.line_height() * row as f32);
        }
    })
    .unwrap();
}

/// The cold estimate puts the extra rows on the row of the formula, so a
/// paragraph is counted at most a text wrap off what shaping makes of it.
#[gpui_kit::test]
fn a_paragraph_with_one_tall_formula_is_counted_by_its_row(cx: &mut TestAppContext) {
    let window = open(cx);
    let text = format!("intro\n\n{}\n\nlast\n", long_paragraph());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let style = window.text_style();
        let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
        let look = (&font, size, style.color);
        let doc = DocumentCache::new(&text);
        let mut cache = RowCache::empty();
        rows::rebuild_for_test(&mut cache, &text, &doc, (0, false), px(WIDTH), look, window);
        let cold = rows::count_at(&cache, 2);
        let shaped =
            rows::shaped_count_for_test(&text, &doc, 2, (0, false), px(WIDTH), look, window);
        assert!(
            (0..=1).contains(&(shaped as i64 - cold)),
            "cold {cold}, shaped {shaped}"
        );
        // Not the old whole-line growth, which made every row tall.
        let rows_of_text = (text.lines().nth(2).unwrap().chars().count() / 90).max(1);
        assert!(
            shaped < rows_of_text * 2,
            "{shaped} rows for about {rows_of_text} of text"
        );
    })
    .unwrap();
}

/// Clicks land on the text of the rows below a tall one, and a click on the
/// tall row itself lands on it (through real dispatch).
#[gpui_kit::test]
fn a_click_below_a_tall_row_lands_on_its_own_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    // The paragraph is below the caret's line, which shows its source.
    let (text, above) = (long_paragraph(), "intro\n\n");
    let owned = format!("{above}{text}");
    let (window, editor) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, move |_w, cx| {
            cx.new(|cx| Editor::new(&owned, cx))
        })
        .unwrap()
    });
    // Each wrapped row: where its second word starts (x, painted y) and the
    // offset a click there must give.
    let spots = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let width = window.find("editor").bounds().size.width;
            let (wrapped, display, entry) = shaped_paragraph(&text, width, window);
            let lh = window.line_height();
            (0..=wrapped.wrap_boundaries().len())
                .map(|row| {
                    let at = super::justify::row_start(&wrapped, row).unwrap() + 2;
                    let pos = wrapped.position_for_index(at, entry.row).unwrap();
                    let (pos, height) = entry.place(pos);
                    let y = lh * 2. + pos.y + height / 2.;
                    (pos.x, y, above.len() + display.raw_offset_for_display(at))
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(spots.len() >= 4);
    for (row, (x, y, target)) in spots.into_iter().enumerate() {
        // The first row holds the formula and is tall; the others are not.
        cx.update_window(window, |_, window, cx| {
            window.click_at("editor", gpui_kit::point(x + px(1.), y), cx);
        })
        .unwrap();
        let cursor = editor.read_with(cx, |e, _| e.buffer().cursor());
        assert_eq!(
            cursor, target,
            "row {row}: the click should land on the word it hit"
        );
        // Back above the paragraph so it is drawn rendered again.
        cx.update_window(window, |_, window, cx| {
            window.click_at("editor", gpui_kit::point(px(3.), px(3.)), cx);
        })
        .unwrap();
    }
}
