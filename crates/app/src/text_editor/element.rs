use gpui_kit::*;

use super::Editor;

/// A pale, half-transparent highlight — a reasonable Phase 1 default, not yet
/// pulled from the app's theme (`ActiveTheme`/`cx.theme()`, as `gpui-component`
/// itself does for its own selection color).
const SELECTION_HIGHLIGHT: Hsla = Hsla { h: 0.6, s: 0.9, l: 0.6, a: 0.35 };

/// Renders `editor`'s buffer: each `\n`-delimited line shaped and painted, a
/// selection highlight rect behind any selected text, and (while focused) a
/// thin cursor caret. Also registers mouse handling for the current frame
/// (click to place the cursor, drag to select) — keyboard input is the next
/// Phase 1 step.
///
/// Composed from plain `div()` + `canvas()` rather than a hand-implemented
/// `gpui::Element` — the outer `div` gets sizing and focus for free from the
/// exact same builder methods already used throughout `main.rs`; `canvas`
/// only needs to own the actual text/cursor painting and (since it has
/// `bounds` in scope precisely when painting, unlike an event registered
/// outside of paint) the mouse hit-testing math.
pub(super) fn render_editor(
    editor: &Editor,
    editor_entity: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> impl IntoElement {
    // Read straight from `editor` (the `&mut self` `Editor::render` already
    // has), not `editor_entity.read(cx)` — this function runs *as* that
    // entity's own render, so reading through the entity handle here panics
    // ("cannot read while already being updated"). `editor_entity` is only
    // for the mouse closures below, which run later, once rendering has
    // finished and reading/updating the entity is safe again.
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
        .on_key_down(cx.listener(Editor::handle_key_down))
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

                    register_mouse_handling(
                        editor_entity.clone(),
                        text.clone(),
                        bounds,
                        line_height,
                        font.clone(),
                        font_size,
                        focus_handle.clone(),
                        window,
                    );

                    // Only registers while `focus_handle` is actually focused
                    // (checked internally by `handle_input`) — installs this
                    // editor as the platform's text input target so typing
                    // and IME composition reach `input_handler.rs` instead of
                    // (or before falling through to) `.on_key_down`.
                    window.handle_input(&focus_handle, ElementInputHandler::new(bounds, editor_entity.clone()), cx);
                },
            )
            .size_full(),
        )
}

/// Registers this frame's mouse listeners: press to focus and place the
/// cursor (collapsing any selection), drag (move while the button that
/// pressed inside these bounds is still down) to extend it, release to stop
/// dragging. `window.on_mouse_event` listeners last one frame each — this
/// runs on every paint, same as the painting itself.
#[allow(clippy::too_many_arguments)]
fn register_mouse_handling(
    editor_entity: Entity<Editor>,
    text: String,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: Font,
    font_size: Pixels,
    focus_handle: FocusHandle,
    window: &mut Window,
) {
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let text = text.clone();
        let font = font.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
            {
                return;
            }
            let offset = offset_for_position(&text, event.position, bounds, line_height, &font, font_size, window);
            editor_entity.update(cx, |editor, cx| {
                editor.buffer_mut().set_cursor(offset, false);
                editor.is_dragging = true;
                cx.notify();
            });
            focus_handle.focus(window, cx);
        }
    });

    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !editor_entity.read(cx).is_dragging {
                return;
            }
            let offset = offset_for_position(&text, event.position, bounds, line_height, &font, font_size, window);
            editor_entity.update(cx, |editor, cx| {
                editor.buffer_mut().set_cursor(offset, true);
                cx.notify();
            });
        }
    });

    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        editor_entity.update(cx, |editor, cx| {
            if editor.is_dragging {
                editor.is_dragging = false;
                cx.notify();
            }
        });
    });
}

/// The byte offset closest to `position` (window coordinates): the line
/// implied by its `y`, then the closest char boundary on that line to its
/// `x`, via `LineLayout::closest_index_for_x` (through `ShapedLine`'s
/// `Deref`). Reshapes the target line rather than reusing a shaped line from
/// painting — cheap for one line, and avoids persisting shaped lines across
/// frames just for this.
fn offset_for_position(
    text: &str,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    window: &mut Window,
) -> usize {
    let relative_y = (position.y - bounds.origin.y).max(px(0.));
    let target_line = (relative_y / line_height) as usize;

    let mut line_start = 0usize;
    for (i, line_text) in text.split('\n').enumerate() {
        let line_end = line_start + line_text.len();
        if i == target_line {
            let run = TextRun {
                len: line_text.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(line_text.to_string().into(), font_size, &[run], None);
            let relative_x = (position.x - bounds.origin.x).max(px(0.));
            return line_start + shaped.closest_index_for_x(relative_x);
        }
        line_start = line_end + 1;
    }
    // Past the last line (clicked below all the text): land at the very end.
    text.len()
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
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, Pixels, Point, TestAppContext, WindowOptions, point, px};

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

    #[gpui_kit::test]
    fn clicking_focuses_the_editor_and_places_the_cursor_near_the_click(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge, not exactly at x=0 — target_position's own
            // click-at-center default would otherwise land mid-string.
            window.click_at("editor", point(px(1.), px(1.)), cx);
        })
        .unwrap();

        let focused = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("editor").focused()
        }).unwrap();
        assert_eq!(focused, Some(true), "clicking the editor should focus it");

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert!(cursor <= 2, "clicking near the start should place the cursor near byte 0, got {cursor}");
    }

    #[gpui_kit::test]
    fn dragging_selects_a_range_of_text(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");

        let bounds = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds()
            })
            .unwrap();
        let from: Point<Pixels> = bounds.origin + point(px(1.), px(1.));
        let to: Point<Pixels> = bounds.origin + point(bounds.size.width - px(1.), px(1.));

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.drag(from, to, cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        let selection = selection.expect("dragging across the text should select some of it");
        assert!(selection.start < selection.end);
        assert!(
            selection.end - selection.start >= "hello world".len() / 2,
            "dragging nearly the whole width should select most of the text, got {selection:?}"
        );
    }

    /// Clicks the editor to focus it, placing the cursor wherever the click
    /// lands — a specific position isn't needed since every test below moves
    /// it explicitly afterward (Home/End/select-all) before asserting.
    fn focus_editor(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        window.render_frame(cx);
        window.click("editor", cx);
    }

    #[gpui_kit::test]
    fn typing_inserts_text_at_the_cursor(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.input("hello", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello");
    }

    #[gpui_kit::test]
    fn backspace_deletes_the_previous_character(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("backspace", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hell");
    }

    #[gpui_kit::test]
    fn home_and_shift_right_select_from_the_line_start(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("home", cx);
            window.press("shift-right", cx);
            window.press("shift-right", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0..2));
    }

    #[gpui_kit::test]
    fn enter_inserts_a_newline(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("enter", cx);
            window.input("world", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello\nworld");
    }

    #[gpui_kit::test]
    fn ctrl_a_selects_everything(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0.."hello world".len()));
    }

    #[gpui_kit::test]
    fn typing_replaces_an_active_selection(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.input("x", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "x");
    }

    #[gpui_kit::test]
    fn copy_then_paste_duplicates_the_selection(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            window.press("end", cx);
            window.input(" ", cx);
            window.press("ctrl-v", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello world hello world");
    }

    #[gpui_kit::test]
    fn cut_removes_the_selection_and_copies_it(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-x", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "", "cut should remove the selected text");

        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-v", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello world", "pasting should bring the cut text back");
    }
}
