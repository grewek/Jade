//! A tall formula makes its line taller wherever it is drawn, the cold row
//! estimate agrees with shaping there, and a region's height follows its
//! lines (#23). Where a formula is not drawn (a table cell shows its source)
//! the line stays one row.

use super::super::Editor;
use super::region::shape_region;
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::layout::LineState;
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
