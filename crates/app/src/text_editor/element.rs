use gpui_kit::*;

use super::Editor;

/// A pale, half-transparent highlight — a reasonable Phase 1 default, not yet
/// pulled from the app's theme (`ActiveTheme`/`cx.theme()`, as `gpui-component`
/// itself does for its own selection color).
const SELECTION_HIGHLIGHT: Hsla = Hsla { h: 0.6, s: 0.9, l: 0.6, a: 0.35 };

/// Renders `editor`'s buffer: each `\n`-delimited line shaped and painted, a
/// selection highlight rect behind any selected text, and (while focused) a
/// thin cursor caret. No mouse/keyboard wiring yet — `canvas`'s own prepaint
/// closure is a no-op; that lands in later Phase 1 steps.
///
/// Composed from plain `div()` + `canvas()` rather than a hand-implemented
/// `gpui::Element` — the outer `div` gets sizing, focus, and (later) mouse/key
/// handling for free from the exact same builder methods already used
/// throughout `main.rs`; `canvas` only needs to own the actual text/cursor
/// painting.
pub(super) fn render_editor(
    editor: &Editor,
    window: &mut Window,
    _cx: &mut Context<Editor>,
) -> impl IntoElement {
    let buffer = editor.buffer();
    let text = buffer.text().to_string();
    let cursor = buffer.cursor();
    let selection = buffer.selection();
    let focus_handle = editor.focus_handle.clone();

    let line_height = window.line_height();
    let line_count = text.split('\n').count().max(1);

    let text_style = window.text_style();
    let font = text_style.font();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = text_style.color;

    div()
        .id("editor")
        .test_support()
        .w_full()
        .h(line_height * line_count as f32)
        .track_focus(&focus_handle)
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, cx| {
                    paint_lines(
                        &text,
                        cursor,
                        selection.clone(),
                        focus_handle.is_focused(window),
                        bounds,
                        line_height,
                        &font,
                        font_size,
                        text_color,
                        window,
                        cx,
                    );
                },
            )
            .size_full(),
        )
}

#[allow(clippy::too_many_arguments)]
fn paint_lines(
    text: &str,
    cursor: usize,
    selection: Option<std::ops::Range<usize>>,
    show_cursor: bool,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let mut line_start = 0usize;
    for (i, line_text) in text.split('\n').enumerate() {
        let line_origin = bounds.origin + point(px(0.), line_height * i as f32);
        let line_range = line_start..line_start + line_text.len();

        let run =
            TextRun { len: line_text.len(), font: font.clone(), color: text_color, background_color: None, underline: None, strikethrough: None };
        let shaped = window.text_system().shape_line(line_text.to_string().into(), font_size, &[run], None);

        if let Some(selection) = &selection {
            let start = selection.start.max(line_range.start);
            let end = selection.end.min(line_range.end);
            if start < end {
                let x0 = shaped.split_at(start - line_start).0.width();
                let x1 = shaped.split_at(end - line_start).0.width();
                window.paint_quad(fill(
                    Bounds { origin: line_origin + point(x0, px(0.)), size: size(x1 - x0, line_height) },
                    SELECTION_HIGHLIGHT,
                ));
            }
        }

        let _ = shaped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);

        if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
            let x = shaped.split_at(cursor - line_start).0.width();
            window.paint_quad(fill(
                Bounds { origin: line_origin + point(x, px(0.)), size: size(px(1.), line_height) },
                text_color,
            ));
        }

        line_start = line_range.end + 1; // +1 for the '\n' this line was split on
    }
}

// Deliberately not `use super::*`/`use gpui_kit::*` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows plain `#[test]` and makes `#[gpui_kit::test]`'s own expansion
// recurse infinitely (see the project-wide note on this in `main.rs`).
#[cfg(test)]
mod tests {
    use super::super::Editor;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};

    fn open_editor(cx: &mut TestAppContext, text: &'static str) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
                cx.new(|cx| Editor::new(text, cx))
            })
            .expect("open test window")
        })
    }

    #[gpui_kit::test]
    fn renders_without_panicking_for_various_buffer_states(cx: &mut TestAppContext) {
        for text in ["", "hello", "line1\nline2\nline3", "trailing newline\n"] {
            let (window, _editor) = open_editor(cx, text);
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("editor").is_some(), "text={text:?}");
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn height_scales_with_line_count(cx: &mut TestAppContext) {
        let (one_line_window, _editor) = open_editor(cx, "just one line");
        let one_line_height = cx
            .update_window(one_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        let (three_line_window, _editor) = open_editor(cx, "line1\nline2\nline3");
        let three_line_height = cx
            .update_window(three_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        assert_eq!(three_line_height, one_line_height * 3., "three lines should be exactly 3x as tall as one");
    }

    #[gpui_kit::test]
    fn renders_with_a_selection_without_panicking(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(0, false);
            editor.buffer_mut().set_cursor(5, true);
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }
}
