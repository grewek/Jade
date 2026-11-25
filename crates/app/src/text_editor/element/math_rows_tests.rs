//! A row holding a tall formula is taller, and painting, hit-testing and the
//! arrow keys agree on it (#15).

use super::super::Editor;
use super::LineLayout;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, Keystroke, Pixels, TestAppContext, WindowOptions, point,
    px,
};

const TALL: &str = "$\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}$";
const SMALL: &str = "$x^2$";

fn open(cx: &mut TestAppContext, text: &'static str) -> (AnyWindowHandle, Entity<Editor>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        cx.bind_keys(crate::text_editor::key_bindings());
    });
    let opened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
            cx.new(|cx| Editor::new(text, cx))
        })
        .expect("open test window")
    });
    // The caret stays on line 0, so the formula on line 1 is drawn, not raw.
    opened
        .1
        .update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
    opened
}

/// The editor after two frames (row counts are made exact while painting),
/// with its layout and the editor's row height.
fn settle(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    editor: &Entity<Editor>,
) -> (LineLayout, Pixels) {
    let line_height = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.line_height()
        })
        .unwrap();
    let view = editor
        .read_with(cx, |editor, _| editor.last_view.clone())
        .unwrap();
    (view.layout.clone(), line_height)
}

fn note(formula: &str) -> &'static str {
    Box::leak(format!("intro\nsee {formula} here\nlast line").into_boxed_str())
}

#[gpui_kit::test]
fn a_tall_formula_makes_its_line_taller(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, note(TALL));
    let (layout, lh) = settle(cx, window, &editor);
    let rows = (layout.height(1) / lh).round();
    assert!(rows >= 2., "the matrix line is {rows} rows");
    assert!(
        (layout.height(1) / lh - rows).abs() < 1e-3,
        "whole rows only"
    );
    assert_eq!(layout.height(0), lh);
    assert_eq!(layout.height(2), lh);
}

#[gpui_kit::test]
fn a_small_formula_keeps_the_normal_row(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, note(SMALL));
    let (layout, lh) = settle(cx, window, &editor);
    assert_eq!(layout.height(1), lh);
}

#[gpui_kit::test]
fn clicks_land_on_the_line_under_them_either_side_of_a_tall_row(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, _) = settle(cx, window, &editor);
    let (top, bottom) = (layout.cumulative_at(1), layout.cumulative_at(2));
    // A click moves the caret onto its line, which then draws raw and shrinks
    // back to one row: put the caret back before each probe.
    let line = |at: Pixels, cx: &mut TestAppContext| {
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
        settle(cx, window, &editor);
        cx.update_window(window, |_, window, cx| {
            window.click_at("editor", point(px(1.), at), cx);
        })
        .unwrap();
        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        text[..cursor].matches('\n').count()
    };
    assert_eq!(line(top + px(2.), cx), 1, "just inside the tall row");
    assert_eq!(line(bottom - px(2.), cx), 1, "just above the next line");
    assert_eq!(line(bottom + px(2.), cx), 2, "just inside the next line");
}

#[gpui_kit::test]
fn arrow_up_from_the_line_below_goes_to_the_tall_line(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, lh) = settle(cx, window, &editor);
    let below = layout.cumulative_at(2) + lh / 2.;
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), below), cx);
        window.render_frame(cx);
        window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
    assert_eq!(text[..cursor].matches('\n').count(), 1);
}

#[gpui_kit::test]
fn arrow_down_from_the_tall_line_goes_to_the_line_below(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, _) = settle(cx, window, &editor);
    let inside = layout.cumulative_at(1) + px(2.);
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), inside), cx);
        window.render_frame(cx);
        window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
    assert_eq!(text[..cursor].matches('\n').count(), 2);
}

const BLOCK: &str = "intro\n$$\n\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}\n$$\nlast line";

#[gpui_kit::test]
fn a_block_is_one_tall_block_of_rows_over_its_first_line(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, BLOCK);
    let (layout, lh) = settle(cx, window, &editor);
    let rows = (layout.height(1) / lh).round();
    assert!(rows >= 2., "the block is {rows} rows");
    assert_eq!((layout.height(2), layout.height(3)), (px(0.), px(0.)));
    assert_eq!((layout.height(0), layout.height(4)), (lh, lh));
}

#[gpui_kit::test]
fn the_caret_in_a_block_shows_its_source_line_by_line(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, BLOCK);
    let (layout, lh) = settle(cx, window, &editor);
    // A click (which focuses the editor) puts the caret in the block.
    let inside = layout.cumulative_at(1) + lh;
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), inside), cx);
    })
    .unwrap();
    let (layout, _) = settle(cx, window, &editor);
    for line in 0..5 {
        assert_eq!(layout.height(line), lh, "line {line}");
    }
    // Out again: it is a formula once more.
    editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
    let (layout, _) = settle(cx, window, &editor);
    assert!(layout.height(1) >= lh * 2.);
    assert_eq!(layout.height(2), px(0.));
}

#[gpui_kit::test]
fn clicking_a_rendered_block_puts_the_caret_in_it(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, BLOCK);
    let (layout, lh) = settle(cx, window, &editor);
    let inside = layout.cumulative_at(1) + lh;
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), inside), cx);
    })
    .unwrap();
    let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
    let line = BLOCK[..cursor].matches('\n').count();
    assert!((1..=3).contains(&line), "caret on line {line}");
}

#[gpui_kit::test]
fn a_block_that_does_not_parse_stays_source(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, "intro\n$$\n\\frac{1\n$$\nlast line");
    let (layout, lh) = settle(cx, window, &editor);
    for line in 0..5 {
        assert_eq!(layout.height(line), lh, "line {line}");
    }
}

/// A formula that grows its row must already be counted when a note is first
/// laid out: left to paint, it corrects the height one stretch at a time as
/// the note scrolls, and the scrollbar creeps (#20).
#[gpui_kit::test]
fn the_cold_estimate_counts_the_rows_a_tall_formula_adds(cx: &mut TestAppContext) {
    use crate::text_editor::doc::DocumentCache;
    use crate::text_editor::rows::{self as rowcache, Inputs, RowCache};

    let text = format!(
        "intro\nsee {TALL} here\nplain\n> quote {TALL}\nsmall {SMALL} end\n{TALL} {TALL}\nlast"
    );
    let (window, _editor) = open(cx, Box::leak(String::new().into_boxed_str()));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let width = px(800.);
        let doc = DocumentCache::new(&text);
        let mut rows = RowCache::empty();
        rowcache::rebuild_for_test(
            &mut rows,
            &text,
            &doc,
            (0, false),
            width,
            (&font, font_size, style.color),
            window,
        );
        let inp = Inputs {
            text: &text,
            cursor: 0,
            focused: false,
            width,
            font: &font,
            font_size,
            text_color: style.color,
        };
        for i in 0..doc.line_count() {
            let exact = rowcache::exact_row_count(&inp, &doc, i, window) as i64;
            assert_eq!(rowcache::count_at(&rows, i), exact, "line {i}");
        }
        assert!(rowcache::count_at(&rows, 1) >= 2, "the matrix line is tall");
        assert_eq!(rowcache::count_at(&rows, 4), 1, "a small formula is not");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_note_with_tall_formulas_is_painted_without_late_corrections(cx: &mut TestAppContext) {
    let text = (0..40)
        .map(|i| format!("line {i} {TALL} end"))
        .collect::<Vec<_>>()
        .join("\n");
    let (window, editor) = open(cx, Box::leak(text.into_boxed_str()));
    // Frames after the width is known: counts come from the estimate and the
    // verified window, so painting has nothing left to fix.
    let (layout, _) = settle(cx, window, &editor);
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(layout.trace_state().2, 0, "late corrections");
}

/// The estimate reads the font's metrics; a shaped line must report the same
/// or the estimate and the painted rows would round differently.
#[gpui_kit::test]
fn font_metrics_match_what_a_shaped_line_reports(cx: &mut TestAppContext) {
    use crate::text_editor::layout::mono_ascent_descent;
    use gpui_kit::TextRun;

    let (window, _editor) = open(cx, "");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let style = window.text_style();
        let font = style.font();
        let size = style.font_size.to_pixels(window.rem_size());
        let run = TextRun {
            len: 1,
            font: font.clone(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line("0".into(), size, &[run], None);
        let (ascent, descent) = mono_ascent_descent(&font, size, window);
        assert_eq!(ascent, f32::from(shaped.ascent));
        assert_eq!(descent, f32::from(shaped.descent));
    })
    .unwrap();
}
