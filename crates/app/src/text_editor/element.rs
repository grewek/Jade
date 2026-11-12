use gpui_kit::*;

use super::block_preview::LineRole;
use super::live_preview::{
    build_display_line, build_display_line_in_context, classify_block_context, hidden_line, DisplayLine,
    InlineStyle, LineContext, LineIndent,
};
use super::Editor;

/// A pale, half-transparent highlight — a reasonable Phase 1 default, not yet
/// pulled from the app's theme (`ActiveTheme`/`cx.theme()`, as `gpui-component`
/// itself does for its own selection color).
const SELECTION_HIGHLIGHT: Hsla = Hsla { h: 0.6, s: 0.9, l: 0.6, a: 0.35 };

/// Live Preview colors for inline code, wikilink, and blockquote/list-marker
/// text — same "flat constant, not yet pulled from the theme" Phase 1
/// compromise as `SELECTION_HIGHLIGHT` above.
const CODE_COLOR: Hsla = Hsla { h: 0.75, s: 0.2, l: 0.55, a: 1.0 };
const WIKILINK_COLOR: Hsla = Hsla { h: 0.58, s: 0.65, l: 0.6, a: 1.0 };
const MUTED_COLOR: Hsla = Hsla { h: 0.0, s: 0.0, l: 0.6, a: 1.0 };

/// Pixel width of one structural indent level (see
/// `live_preview::LineIndent`) — applied identically by `paint_lines`,
/// `offset_for_position`, and (via [`indent_px`]) `input_handler.rs`'s
/// `bounds_for_range`, so a line's painted position and its hit-testing/IME
/// math always agree.
const INDENT_UNIT: Pixels = px(20.);

/// The left inset, within an indent level's own lane, that a blockquote's
/// quote bar is painted at — see `paint_lines`.
const QUOTE_BAR_INSET: Pixels = px(4.);
const QUOTE_BAR_WIDTH: Pixels = px(2.);

/// The pixel x-offset `level` structural indent levels shifts a line's
/// content by — `level` is [`LineIndent::level`], `0` for a line with no
/// indent.
pub(super) fn indent_px(level: u8) -> Pixels {
    INDENT_UNIT * level as f32
}

/// Whether the line spanning `line_range` should render raw/unstyled rather
/// than with Live Preview markup hidden: only while `focused` (the editor
/// actually has focus — a freshly opened note has an unfocused editor whose
/// `EditorBuffer::cursor()` still defaults to byte `0`, which would
/// otherwise permanently pin line 0 as "active" even though no editing is
/// happening) *and* `cursor` sits inside `line_range` (inclusive of both
/// ends, so a cursor resting right at a line's last byte, before its `\n`,
/// still counts).
pub(super) fn is_active_line(line_range: &std::ops::Range<usize>, cursor: usize, focused: bool) -> bool {
    focused && cursor >= line_range.start && cursor <= line_range.end
}

/// Scrolls `scroll_handle`'s tracked container (`main.rs`'s note-editor-pane
/// div — see `Editor::scroll_handle`'s doc comment) just enough to bring the
/// cursor's line fully into view, if it currently isn't. `content_bounds` is
/// this element's own (already scroll-shifted) on-screen bounds, the same
/// `bounds` `paint_lines`/mouse hit-testing use. Adding `line_height *
/// cursor_line_index` to its origin gives the cursor line's actual current
/// screen position, directly comparable against `scroll_handle.bounds()`
/// (the viewport's fixed on-screen bounds, which don't move as you scroll).
///
/// A no-op if the cursor's line is already fully visible, or before the
/// container has been laid out at least once (`scroll_handle.bounds()` is
/// zero-sized until then).
fn autoscroll_to_cursor(scroll_handle: &ScrollHandle, content_bounds: Bounds<Pixels>, line_height: Pixels, cursor_line_index: usize) {
    let viewport = scroll_handle.bounds();
    if viewport.size.height <= px(0.) {
        return;
    }
    let cursor_top = content_bounds.origin.y + line_height * cursor_line_index as f32;
    let cursor_bottom = cursor_top + line_height;
    let viewport_top = viewport.origin.y;
    let viewport_bottom = viewport_top + viewport.size.height;

    let mut offset = scroll_handle.offset();
    if cursor_top < viewport_top {
        offset.y += viewport_top - cursor_top;
    } else if cursor_bottom > viewport_bottom {
        offset.y += viewport_bottom - cursor_bottom;
    } else {
        return;
    }
    // `ScrollHandle::max_offset()` is a positive magnitude (how much
    // scrollable overflow there is), not the offset's own lower bound — the
    // valid offset range is `[-max_offset, 0]`, same convention `gpui-pre`'s
    // own scroll-clamping code uses internally.
    let max_offset = scroll_handle.max_offset();
    offset.y = offset.y.clamp(-max_offset.y, px(0.));
    scroll_handle.set_offset(offset);
}

/// The `Font` to paint a [`DisplaySegment`](super::live_preview::DisplaySegment)'s
/// visible text in, derived from `base` (the editor's ambient mono font) —
/// only weight/style vary per [`InlineStyle`], not family or size, so every
/// run in a shaped line still lines up on the same baseline/metrics.
fn font_for_style(style: InlineStyle, base: &Font) -> Font {
    let mut font = base.clone();
    match style {
        InlineStyle::Bold | InlineStyle::Heading(_) => font.weight = FontWeight::BOLD,
        InlineStyle::Italic | InlineStyle::Blockquote(_) => font.style = FontStyle::Italic,
        InlineStyle::Plain
        | InlineStyle::Code
        | InlineStyle::Strikethrough
        | InlineStyle::WikiLink
        | InlineStyle::ListMarker => {}
    }
    font
}

/// The color to paint a visible segment's text in — distinct colors for
/// inline code, wikilinks, and blockquote/list-marker text, `base` (the
/// editor's ambient text color) otherwise.
fn color_for_style(style: InlineStyle, base: Hsla) -> Hsla {
    match style {
        InlineStyle::Code => CODE_COLOR,
        InlineStyle::WikiLink => WIKILINK_COLOR,
        InlineStyle::Blockquote(_) | InlineStyle::ListMarker => MUTED_COLOR,
        InlineStyle::Plain | InlineStyle::Bold | InlineStyle::Italic | InlineStyle::Heading(_) | InlineStyle::Strikethrough => base,
    }
}

/// The strikethrough line to paint over a visible segment's text, or `None`
/// for every style but [`InlineStyle::Strikethrough`] itself.
fn strikethrough_for_style(style: InlineStyle, color: Hsla) -> Option<StrikethroughStyle> {
    match style {
        InlineStyle::Strikethrough => Some(StrikethroughStyle { thickness: px(1.), color: Some(color) }),
        _ => None,
    }
}

/// Shapes `line`'s Live Preview display text into one multi-styled
/// `ShapedLine`, alongside the [`DisplayLine`] mapping needed to convert a
/// raw buffer offset into an x position in it (or back). `role` (from
/// `block_preview::classify_lines`, computed once per document) picks
/// between three cases:
/// - [`LineRole::FenceContent`]: always literal (`build_display_line(line,
///   true)`, i.e. the same "show it verbatim" path an active line takes) —
///   code fence content is never Markdown, so `active` doesn't apply to it.
/// - [`LineRole::FenceDelimiter`]: raw when `active`, else [`hidden_line`]
///   — the fence marker itself has no "remainder" to show once hidden,
///   unlike every other hidden-prefix case in `live_preview.rs`.
/// - [`LineRole::Plain`]: `build_display_line_in_context(line, active,
///   block_ctx)` — `block_ctx` (from `live_preview::classify_block_context`,
///   also computed once per document) resolves nested-list depth and lazy
///   continuation, neither of which `line` alone carries enough information
///   for.
#[allow(clippy::too_many_arguments)]
pub(super) fn shape_display_line(
    line: &str,
    role: LineRole,
    block_ctx: LineContext,
    active: bool,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> (ShapedLine, DisplayLine) {
    let mut display_line = match role {
        LineRole::FenceContent => build_display_line(line, true),
        LineRole::FenceDelimiter if !active => hidden_line(line),
        LineRole::FenceDelimiter => build_display_line(line, active),
        LineRole::Plain => build_display_line_in_context(line, active, block_ctx),
    };
    // Fenced code content is never Markdown (see this function's own doc
    // comment above) — so even if it happens to start with `>` or `- `,
    // that's not a real blockquote/list line and shouldn't indent as one.
    if matches!(role, LineRole::FenceContent) {
        display_line.indent = LineIndent::None;
    }

    let mut runs: Vec<TextRun> = display_line
        .segments
        .iter()
        .filter(|seg| !seg.is_hidden())
        .map(|seg| {
            let color = color_for_style(seg.style, text_color);
            TextRun {
                len: seg.display.len(),
                font: font_for_style(seg.style, font),
                color,
                background_color: None,
                underline: None,
                strikethrough: strikethrough_for_style(seg.style, color),
            }
        })
        .collect();
    if runs.is_empty() {
        runs.push(TextRun { len: 0, font: font.clone(), color: text_color, background_color: None, underline: None, strikethrough: None });
    }

    let shaped = window.text_system().shape_line(display_line.display_text.clone().into(), font_size, &runs, None);
    (shaped, display_line)
}

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
    let cursor_line_index = text[..cursor].matches('\n').count();
    let selection = buffer.selection();
    let focus_handle = editor.focus_handle.clone();
    let scroll_handle = editor.scroll_handle.clone();
    let last_autoscrolled_cursor = editor.last_autoscrolled_cursor.clone();
    // Computed once per paint (not per line) since it needs the whole
    // document to tell whether a given line sits inside an open code
    // fence — see `block_preview`'s module doc.
    let line_roles = super::block_preview::classify_lines(&text);
    // Same "whole document, once per paint" reasoning as `line_roles` above
    // — nested-list depth and lazy continuation both depend on prior lines,
    // see `live_preview::classify_block_context`'s own doc comment.
    let block_contexts = classify_block_context(&text);

    let line_height = window.line_height();
    let line_count = text.split('\n').count().max(1);

    let text_style = window.text_style();
    // The note editor's own font, not the ambient `text_style`'s — only the
    // bundled mono family has real Bold/Italic/BoldItalic faces (the Sans UI
    // family only has Regular/Italic), which Live Preview's bold/italic runs
    // need. Fixes the font-family gap `text_editor`'s module doc previously
    // called out as a deferred cosmetic cut, now that this code path
    // actually needs the distinction.
    let mut font = text_style.font();
    font.family = gpui_kit::component::Theme::global(cx).mono_font_family.clone();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = text_style.color;

    div()
        .id("editor")
        .test_support()
        .w_full()
        .h(line_height * line_count as f32)
        .track_focus(&focus_handle)
        // Must out-rank `gpui_kit::init`'s global `Root` component, which
        // binds plain "tab" to its own focus-next action in a "Root"
        // context wrapping the whole app — see `Editor::handle_insert_tab`'s
        // doc comment for why an `on_key_down` match arm alone can't do this.
        .key_context("TextEditor")
        .on_action(cx.listener(Editor::handle_insert_tab))
        .on_key_down(cx.listener(Editor::handle_key_down))
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, cx| {
                    let focused = focus_handle.is_focused(window);
                    // Only snap the view back to the cursor the first time
                    // we see this particular cursor position, not on every
                    // repaint — a plain mouse-wheel scroll alone triggers a
                    // repaint too (with the cursor unchanged), and without
                    // this check it would get fought right back to the
                    // cursor's line the moment that line scrolled out of
                    // view. See `last_autoscrolled_cursor`'s own doc comment.
                    if focused && last_autoscrolled_cursor.get() != Some(cursor) {
                        autoscroll_to_cursor(&scroll_handle, bounds, line_height, cursor_line_index);
                        last_autoscrolled_cursor.set(Some(cursor));
                    }
                    paint_lines(
                        &text,
                        &line_roles,
                        &block_contexts,
                        cursor,
                        selection.clone(),
                        focused,
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
                        line_roles.clone(),
                        block_contexts.clone(),
                        cursor,
                        focused,
                        bounds,
                        line_height,
                        font.clone(),
                        font_size,
                        text_color,
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
    line_roles: Vec<LineRole>,
    block_contexts: Vec<LineContext>,
    cursor: usize,
    focused: bool,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: Font,
    font_size: Pixels,
    text_color: Hsla,
    focus_handle: FocusHandle,
    window: &mut Window,
) {
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let text = text.clone();
        let line_roles = line_roles.clone();
        let block_contexts = block_contexts.clone();
        let font = font.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
            {
                return;
            }
            let offset = offset_for_position(
                &text, &line_roles, &block_contexts, cursor, focused, event.position, bounds, line_height, &font,
                font_size, text_color, window,
            );
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
            let offset = offset_for_position(
                &text, &line_roles, &block_contexts, cursor, focused, event.position, bounds, line_height, &font,
                font_size, text_color, window,
            );
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
/// implied by its `y`, then the closest char boundary — in that line's
/// *display* text, i.e. accounting for any Live Preview markup hidden on a
/// non-active line — to its `x`, via `LineLayout::closest_index_for_x`
/// (through `ShapedLine`'s `Deref`), mapped back to a raw buffer offset via
/// `DisplayLine::raw_offset_for_display`. Reshapes the target line rather
/// than reusing a shaped line from painting — cheap for one line, and
/// avoids persisting shaped lines across frames just for this.
#[allow(clippy::too_many_arguments)]
fn offset_for_position(
    text: &str,
    line_roles: &[LineRole],
    block_contexts: &[LineContext],
    cursor: usize,
    focused: bool,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> usize {
    let relative_y = (position.y - bounds.origin.y).max(px(0.));
    let target_line = (relative_y / line_height) as usize;

    let mut line_start = 0usize;
    for (i, line_text) in text.split('\n').enumerate() {
        let line_range = line_start..line_start + line_text.len();
        if i == target_line {
            let active = is_active_line(&line_range, cursor, focused);
            let (shaped, display_line) = shape_display_line(
                line_text, line_roles[i], block_contexts[i], active, font, font_size, text_color, window,
            );
            let relative_x = (position.x - bounds.origin.x - indent_px(display_line.indent.level())).max(px(0.));
            let display_offset = shaped.closest_index_for_x(relative_x);
            return line_start + display_line.raw_offset_for_display(display_offset);
        }
        line_start = line_range.end + 1;
    }
    // Past the last line (clicked below all the text): land at the very end.
    text.len()
}

#[allow(clippy::too_many_arguments)]
fn paint_lines(
    text: &str,
    line_roles: &[LineRole],
    block_contexts: &[LineContext],
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
        let row_origin = bounds.origin + point(px(0.), line_height * i as f32);
        let line_range = line_start..line_start + line_text.len();
        let active = is_active_line(&line_range, cursor, show_cursor);

        let (shaped, display_line) = shape_display_line(
            line_text, line_roles[i], block_contexts[i], active, font, font_size, text_color, window,
        );

        if let LineIndent::Blockquote(depth) = display_line.indent {
            for level in 0..depth {
                let bar_x = row_origin.x + indent_px(level) + QUOTE_BAR_INSET;
                window.paint_quad(fill(
                    Bounds { origin: point(bar_x, row_origin.y), size: size(QUOTE_BAR_WIDTH, line_height) },
                    MUTED_COLOR,
                ));
            }
        }
        let line_origin = row_origin + point(indent_px(display_line.indent.level()), px(0.));

        if let Some(selection) = &selection {
            let start = selection.start.max(line_range.start);
            let end = selection.end.min(line_range.end);
            if start < end {
                let x0 = shaped.split_at(display_line.display_offset_for_raw(start - line_start)).0.width();
                let x1 = shaped.split_at(display_line.display_offset_for_raw(end - line_start)).0.width();
                window.paint_quad(fill(
                    Bounds { origin: line_origin + point(x0, px(0.)), size: size(x1 - x0, line_height) },
                    SELECTION_HIGHLIGHT,
                ));
            }
        }

        let _ = shaped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);

        if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
            let x = shaped.split_at(display_line.display_offset_for_raw(cursor - line_start)).0.width();
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
    use super::super::block_preview::LineRole;
    use super::super::live_preview::{LineContext, LineIndent};
    use super::super::{Editor, InsertTab};
    use super::{is_active_line, shape_display_line};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, EntityInputHandler, KeyBinding, Pixels, Point,
        TestAppContext, WindowOptions, point, px,
    };

    fn open_editor(cx: &mut TestAppContext, text: &'static str) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            // Must out-rank `gpui_kit::init`'s own global "tab" binding —
            // see `Editor::handle_insert_tab`'s doc comment.
            cx.bind_keys([KeyBinding::new("tab", InsertTab, Some("TextEditor"))]);
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
    fn tab_inserts_a_tab_character_instead_of_moving_focus_away(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("tab", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello\t", "tab should insert a tab character, not fall through to a no-op");

        let focused = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").focused()
            })
            .unwrap();
        assert_eq!(focused, Some(true), "tab must not move focus out of the editor");
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

    #[gpui_kit::test]
    fn ctrl_z_undoes_the_last_keystrokes_edit(cx: &mut TestAppContext) {
        // `window.input` dispatches one character per keystroke, and each
        // character arrives through `EntityInputHandler::replace_text_in_range`
        // as its own `buffer.insert` call — so, per this editor's documented
        // "basic" (non-coalescing) undo, one ctrl-z only undoes the single
        // last-typed character, not the whole burst of typing.
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.input(" world", cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello worl", "ctrl-z should undo only the last-typed character");
    }

    #[gpui_kit::test]
    fn ctrl_z_then_ctrl_shift_z_round_trips_a_pasted_insert(cx: &mut TestAppContext) {
        // Pasted text arrives in one `buffer.insert` call, so (unlike typing,
        // see the test above) it's a single undo/redo step.
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            window.press("end", cx);
            window.press("ctrl-v", cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello", "ctrl-z should undo the whole pasted insert in one step");

        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-z", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hellohello", "ctrl-shift-z should redo the paste");
    }

    #[gpui_kit::test]
    fn committing_a_non_combining_ime_composition_does_not_duplicate_the_marked_character(
        cx: &mut TestAppContext,
    ) {
        // Reproduces a real dead-key sequence (e.g. "^" or "`" on layouts where
        // they're dead keys): the platform IME first shows a provisional marked
        // preview via `replace_and_mark_text_in_range`, then — once the next
        // keystroke doesn't combine with it (e.g. pressing space) — commits via
        // `replace_text_in_range` with `range_utf16: None`. That commit must
        // replace the marked preview, not insert next to it.
        let (window, editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, "^", None, window, cx);
            });
        })
        .unwrap();

        let marked_preview = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(marked_preview, "^", "the dead-key preview should appear immediately");

        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(None, "^", window, cx);
            });
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "^", "committing the composition must replace the preview, not duplicate it");
    }

    #[gpui_kit::test]
    fn ctrl_right_jumps_to_the_end_of_the_current_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
            window.press("ctrl-right", cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(cursor, 5, "ctrl-right should stop at the end of \"hello\"");
    }

    #[gpui_kit::test]
    fn ctrl_left_jumps_to_the_start_of_the_current_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(9, false));
            window.press("ctrl-left", cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(cursor, 6, "ctrl-left should stop at the start of \"world\"");
    }

    #[gpui_kit::test]
    fn ctrl_shift_right_extends_the_selection_by_a_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
            window.press("ctrl-shift-right", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0..5), "ctrl-shift-right should select through the end of \"hello\"");
    }

    #[gpui_kit::test]
    fn an_unfocused_freshly_opened_editor_still_previews_markup(cx: &mut TestAppContext) {
        let text = "# Heading";
        let (window, editor) = open_editor(cx, text);
        // Never focused — `EditorBuffer::new`'s cursor defaults to byte 0,
        // which sits inside this (only) line's range. Before gating
        // activation on focus too, that alone made the line "active" and
        // rendered it raw even with nothing being edited: opening a note
        // (without clicking into its editor) left its markup unrendered.
        let heading_text_start = text.find("Heading").unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge; the "# " marker should be hidden, so this
            // should land right after it, not at raw byte 0.
            window.click_at("editor", point(px(1.), px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(
            cursor, heading_text_start,
            "a freshly opened, unfocused editor should still preview markup, hiding the heading's \"# \""
        );
    }

    #[gpui_kit::test]
    fn renders_live_preview_constructs_without_panicking(cx: &mut TestAppContext) {
        let text = "# Heading\nsome **bold** and *italic* and `code`\nsee [[Other Note|here]]\nactive line";
        let (window, editor) = open_editor(cx, text);
        // Cursor on the last line, so every other line renders previewed
        // (hidden markup) rather than raw.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(text.len(), false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_a_previewed_heading_lands_past_its_hidden_marker_and_activates_it(cx: &mut TestAppContext) {
        let text = "first line\n# Heading";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 ("# Heading") renders previewed
        // (its "# " hidden, showing just "Heading").
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge of the second rendered row.
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let heading_text_start = text.find("Heading").unwrap();
        assert_eq!(
            cursor, heading_text_start,
            "clicking near the start of the previewed heading should land right after its hidden \"# \", not before it"
        );

        // Having moved the cursor onto that line, a re-render should now
        // show it raw rather than previewed.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        // The click also focused the editor (covered separately by
        // `clicking_focuses_the_editor_and_places_the_cursor_near_the_click`).
        let active = editor.read_with(cx, |editor, _| is_active_line(&(11..text.len()), editor.buffer().cursor(), true));
        assert!(active, "the line the cursor landed on should now be active");
    }

    #[gpui_kit::test]
    fn clicking_a_previewed_list_item_lands_past_its_hidden_marker(cx: &mut TestAppContext) {
        let text = "first line\n- list item";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 ("- list item") renders
        // previewed: its "- " hidden behind a bullet glyph.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge of the second rendered row, i.e. on the
            // bullet glyph itself.
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let item_text_start = text.find("list item").unwrap();
        assert_eq!(
            cursor, item_text_start,
            "clicking the bullet glyph should land right after the hidden \"- \" marker, not before it"
        );
    }

    #[gpui_kit::test]
    fn blockquote_indentation_shifts_the_whole_line_right(cx: &mut TestAppContext) {
        let text = "first line\n> quoted content";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 renders previewed: its "> " is
        // hidden entirely (zero display width), unlike a list marker's
        // replacement bullet — so without the indent fix, the shaped text
        // would start right at the row's left edge.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // x=15 sits inside the blockquote's indent gutter (< INDENT_UNIT,
            // 20px) but, with no hidden-prefix width to absorb it the way a
            // list's bullet does, would already land partway into "quoted
            // content"'s shaped text without the indent shift.
            window.click_at("editor", point(px(15.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let content_start = text.find("quoted").unwrap();
        assert_eq!(
            cursor, content_start,
            "clicking inside the indent gutter should land at the very start of the quoted content"
        );
    }

    #[gpui_kit::test]
    fn renders_nested_lists_and_lazy_continuation_without_panicking(cx: &mut TestAppContext) {
        // A wiring smoke test for `classify_block_context`'s `Vec<LineContext>`
        // being indexed in step with `text.split('\n')` and `line_roles` —
        // an off-by-one there would panic (index out of bounds) rather than
        // just render wrong, so this alone is a meaningful regression net
        // on top of `live_preview`'s own exhaustive pure-logic tests.
        let text = "1. outer\n   - inner\n     still inner, lazy continuation\n> quoted\nlazy quote continuation";
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(text.len(), false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_nested_list_marker_indents_deeper_than_its_top_level_parent(cx: &mut TestAppContext) {
        let text = "first line\n1. outer\n   - inner";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so both list lines render previewed.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // x=35 sits inside the nested (depth-2) item's indent gutter
            // (< 2 * INDENT_UNIT, 40px) but *past* a top-level item's own
            // gutter (>= INDENT_UNIT, 20px) — if `classify_block_context`'s
            // resolved depth of 2 weren't actually reaching this line (e.g.
            // an indexing bug silently left it at `build_display_line`'s
            // own depth-1 default), this would land inside "inner"'s
            // shaped text instead of at its very start.
            window.click_at("editor", point(px(35.), line_height * 2. + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let inner_start = text.find("inner").unwrap();
        assert_eq!(
            cursor, inner_start,
            "clicking inside the nested item's indent gutter should land at the start of its content"
        );
    }

    #[gpui_kit::test]
    fn fenced_code_content_is_never_inline_scanned_even_on_the_active_line(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let line = "**not bold** [[not a wikilink]]";
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            // `active: true` on its own would normally be enough to show a
            // line verbatim anyway (see `live_preview`'s own tests) — the
            // real thing this test guards is that `FenceContent` forces the
            // same literal treatment regardless of `active`, since a fenced
            // code line must never have its content inline-scanned at all,
            // active or not.
            let (_, display_line) = shape_display_line(
                line, LineRole::FenceContent, LineContext::Plain, true, &font, font_size, text_style.color, window,
            );
            assert_eq!(display_line.display_text, line, "fenced code content must never be treated as Markdown");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn fenced_code_content_starting_with_quote_or_list_syntax_is_never_indented(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            for line in ["> looks like a quote", "- looks like a list item"] {
                let (_, display_line) = shape_display_line(
                    line, LineRole::FenceContent, LineContext::Plain, false, &font, font_size, text_style.color,
                    window,
                );
                assert_eq!(
                    display_line.indent,
                    LineIndent::None,
                    "fenced code content must never indent as a quote/list, even if it looks like one: {line:?}"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_fence_delimiter_line_is_hidden_when_not_active(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let line = "```rust";
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let (_, display_line) = shape_display_line(
                line, LineRole::FenceDelimiter, LineContext::Plain, false, &font, font_size, text_style.color, window,
            );
            assert_eq!(display_line.display_text, "", "an inactive fence delimiter line should show nothing");

            let (_, active_display_line) = shape_display_line(
                line, LineRole::FenceDelimiter, LineContext::Plain, true, &font, font_size, text_style.color, window,
            );
            assert_eq!(active_display_line.display_text, line, "the active fence delimiter line should show its raw text");
        })
        .unwrap();
    }
}
