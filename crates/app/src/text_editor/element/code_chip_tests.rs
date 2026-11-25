//! Inline code chips: one run per row the span's text is on, from the first
//! character's own place (#58 E1).

use super::super::Editor;
use super::code_chip::style_runs;
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::layout::{LineState, shape_display_line};
use gpui_kit::{AppContext, Font, TestAppContext, WindowOptions, px};

fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Font) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, _e) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new("", cx))
        })
        .unwrap()
    });
    let family = cx.update(|cx| {
        gpui_kit::component::Theme::global(cx)
            .mono_font_family
            .clone()
    });
    let font = cx
        .update_window(window, |_, window, _| {
            let mut f = window.text_style().font();
            f.family = family;
            f
        })
        .unwrap();
    (window.into(), font)
}

/// The runs of line 1 of `text` at `width`, with the advance of one character
/// and the number of rows the line wrapped to.
fn runs(
    cx: &mut TestAppContext,
    (window, font): (gpui_kit::AnyWindowHandle, &Font),
    text: &str,
    width: f32,
) -> (Vec<super::code_chip::RowRun>, f32, usize) {
    let doc = DocumentCache::new(text);
    cx.update_window(window, |_, window, _| {
        let state = LineState::of(&doc, 1, 0, true);
        let line = &text[doc.line_start_byte(1)..doc.line_end_byte(1)];
        let shaped = shape_display_line(
            line,
            &state,
            font,
            px(16.),
            gpui_kit::black(),
            px(width),
            window,
        );
        let (wrapped, display) = &shaped[0];
        let advance = 16.0 * crate::text_editor::math::MONO_ADVANCE_EM;
        let rows = wrapped.wrap_boundaries().len() + 1;
        let found = style_runs((&state, wrapped, display), px(26.), px(advance), |s| {
            s.style == crate::text_editor::preview::InlineStyle::Code
        });
        (found, advance, rows)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_span_on_one_row_is_one_run_as_wide_as_its_text(cx: &mut TestAppContext) {
    let env = open(cx);
    let (runs, advance, rows) = runs(cx, (env.0, &env.1), "x\nab `code` cd", 900.);
    assert_eq!(rows, 1);
    assert_eq!(runs.len(), 1);
    let width = f32::from(runs[0].to - runs[0].from);
    assert!((width - 4.0 * advance).abs() < 0.5, "{width} vs {advance}");
    assert!((f32::from(runs[0].from) - 3.0 * advance).abs() < 0.5);
}

#[gpui_kit::test]
fn a_span_the_wrap_moves_to_the_next_row_starts_at_its_left_edge(cx: &mut TestAppContext) {
    let env = open(cx);
    // The word cannot share the first row, so the whole span starts the second:
    // GPUI reports its first character at the end of the row before.
    let text = "x\nwords and more `abcdefghijklmnopqrstu` end";
    let (runs, advance, rows) = runs(cx, (env.0, &env.1), text, 230.);
    assert!(rows >= 2, "{rows}");
    assert!(runs.iter().all(|r| r.to > r.from + px(advance)), "{runs:?}");
    let first = runs.first().unwrap();
    assert_eq!(
        first.from,
        px(0.),
        "starts at its row's left edge: {runs:?}"
    );
    let chars: f32 = runs
        .iter()
        .map(|r| f32::from(r.to - r.from) / advance)
        .sum();
    assert!(
        (chars - 21.0).abs() < 0.1,
        "every character in a run: {chars}"
    );
}
