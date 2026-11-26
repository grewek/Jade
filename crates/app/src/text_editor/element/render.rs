//! The editor's element tree: sizing, scrolling, the canvas that paints and
//! registers input each frame, and the right-click menus.

#[allow(unused_imports)]
use super::*;

/// Scrolls `scroll_handle`'s tracked container (`app.rs`'s note-editor-pane
/// div — see `Editor::scroll_handle`'s doc comment) just enough to bring the
/// cursor's line fully into view, if it currently isn't. `content_bounds` is
/// this element's own (already scroll-shifted) on-screen bounds, the same
/// `bounds` `paint_lines`/mouse hit-testing use. Adding `cursor_top` (the
/// cursor's line's accumulated Y offset *within* the document — see
/// `measure_document`, since a wrapped line ahead of it can make this more
/// than `line_height * cursor_line_index`) to that origin gives the cursor
/// line's actual current screen position, directly comparable against
/// `scroll_handle.bounds()` (the viewport's fixed on-screen bounds, which
/// don't move as you scroll). `cursor_height` is that same line's own
/// height (a multiple of `line_height` if it's wrapped across several
/// rows), so a tall wrapped line scrolls its *bottom* row into view rather
/// than stopping as soon as its top row is visible.
///
/// A no-op if the cursor's line is already fully visible, or before the
/// container has been laid out at least once (`scroll_handle.bounds()` is
/// zero-sized until then).
pub(super) fn autoscroll_to_cursor(
    scroll_handle: &ScrollHandle,
    content_bounds: Bounds<Pixels>,
    cursor_top: Pixels,
    cursor_height: Pixels,
) {
    let viewport = scroll_handle.bounds();
    if viewport.size.height <= px(0.) {
        return;
    }
    let cursor_top = content_bounds.origin.y + cursor_top;
    let cursor_bottom = cursor_top + cursor_height;
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

/// The render parameters [`analyze_line_layout`] needs, computed the same
/// way both `render_editor` (every paint) and `Editor::record_edit`
/// (every edit, so [`RowCache`] can be patched
/// synchronously right there rather than deferred to the next paint) need
/// them — pulled out into one function once a second call site needed the
/// exact same computation `render_editor` already did inline.
pub(in crate::text_editor) struct RenderParams {
    pub(in crate::text_editor) font: Font,
    pub(in crate::text_editor) font_size: Pixels,
    pub(in crate::text_editor) text_color: Hsla,
    pub(in crate::text_editor) available_width: Pixels,
    pub(in crate::text_editor) line_height: Pixels,
}

pub(in crate::text_editor) fn render_params(
    editor: &Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> RenderParams {
    let text_style = window.text_style();
    // The note editor's own font, not the ambient `text_style`'s — only the
    // bundled mono family has real Bold/Italic/BoldItalic faces (the Sans UI
    // family only has Regular/Italic), which Live Preview's bold/italic runs
    // need.
    let mut font = text_style.font();
    font.family = gpui_kit::component::Theme::global(cx)
        .mono_font_family
        .clone();
    let font_size = crate::text_editor::settings::body_font_size();
    let text_color = ThemeEngine::color(cx, Role::Text);
    // The editor's own on-screen width isn't known yet before the first
    // layout — `scroll_handle`'s tracked container bounds (the same "last
    // completed layout" value `autoscroll_to_cursor` already relies on)
    // stands in for it, falling back to "don't wrap" (`Pixels::MAX`) only
    // before that first layout, self-correcting the moment it completes.
    let available_width = {
        let width = editor.scroll_handle.bounds().size.width;
        if width > px(0.) { width } else { Pixels::MAX }
    };
    let line_height = crate::text_editor::settings::body_row_height();
    RenderParams {
        font,
        font_size,
        text_color,
        available_width,
        line_height,
    }
}

/// Computes the whole document's total rendered height (estimated — see
/// [`analyze_line_layout`]) and the cursor's own line's accumulated top-Y
/// offset and height — needed by `render_editor` *before* it can construct
/// its element tree (the outer `div`'s own `.h(...)` sizing, and
/// `autoscroll_to_cursor`'s inputs), which is before the canvas paint
/// closure where `paint_lines`/`offset_for_position` do their own (now
/// viewport-bounded, real) shaping. No shaping happens in this function at
/// all — everything here is a handful of O(log n)/O(callouts.len()) lookups
/// into `layout`, which (via `analyze_line_layout`) is almost always already
/// up to date before this paint even starts.
pub(super) fn measure_document(
    layout: &LineLayout,
    cursor: usize,
    line_height: Pixels,
) -> (Pixels, Pixels, Pixels) {
    let total = layout.cumulative_at(layout.line_count());

    let mut cursor_line = layout.line_at(cursor);
    // A caret in a side-by-side region scrolls the whole region into view: only
    // its first line has rows.
    if let Some((region, _)) = layout.doc_cache.borrow().side_by_side(cursor_line) {
        cursor_line = region.start_line;
    }
    let cursor_top = layout.cumulative_at(cursor_line);
    let cursor_height = layout.height(cursor_line).max(line_height);

    (total, cursor_top, cursor_height)
}

/// Renders `editor`'s buffer: each `\n`-delimited line shaped and painted, a
/// selection highlight rect behind any selected text, and (while focused) a
/// thin cursor caret. Also registers mouse handling for the current frame
/// (click to place the cursor, drag to select) — keyboard input is the next
/// Phase 1 step.
///
/// Composed from plain `div()` + `canvas()` rather than a hand-implemented
/// `gpui::Element` — the outer `div` gets sizing and focus for free from the
/// exact same builder methods already used throughout `app.rs`; `canvas`
/// only needs to own the actual text/cursor painting and (since it has
/// `bounds` in scope precisely when painting, unlike an event registered
/// outside of paint) the mouse hit-testing math.
/// A listener running a table shortcut's [`Op`] when its action fires.
macro_rules! table_action {
    ($cx:expr, $action:ident, $op:expr) => {
        $cx.listener(
            |editor, _: &crate::text_editor::editor::$action, window, cx| {
                editor.table_action($op, window, cx)
            },
        )
    };
}

pub(in crate::text_editor) fn render_editor(
    editor: &Editor,
    editor_entity: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> impl IntoElement {
    let render_start = crate::debug_server::frames::now();
    let menu_editor = editor_entity.clone();
    // Read straight from `editor` (the `&mut self` `Editor::render` already
    // has), not `editor_entity.read(cx)` — this function runs *as* that
    // entity's own render, so reading through the entity handle here panics
    // ("cannot read while already being updated"). `editor_entity` is only
    // for the mouse closures below, which run later, once rendering has
    // finished and reading/updating the entity is safe again.
    let buffer = editor.buffer();
    let text = buffer.text();
    let cursor = buffer.cursor();
    let selection = buffer.selection();
    let focus_handle = editor.focus_handle.clone();
    let scroll_handle = editor.scroll_handle.clone();
    let last_autoscrolled_cursor = editor.last_autoscrolled_cursor.clone();
    // Reads the persisted `DocumentCache` (see that module's own doc
    // comment) instead of re-running all four classify_* passes per paint.
    // The paint and mouse closures below are `'static`, so they reach the
    // text and cache through `editor_entity` when they run rather than
    // holding per-frame copies (which cost ~25MB per frame on the stress
    // fixture, the same on every scroll as on every keystroke).
    let document_cache = editor.document_cache.clone();
    let row_height_cache = editor.row_height_cache.clone();

    let focused = focus_handle.is_focused(window);
    let params = render_params(editor, window, cx);
    let RenderParams {
        font,
        font_size,
        text_color,
        available_width,
        line_height,
    } = params;

    // Makes sure `row_height_cache` is up to date (almost always a cheap
    // active-line check — see `RowCache`'s own doc comment, and
    // `Editor::record_edit` for the edit-triggered half of
    // keeping it that way) and wraps it alongside `document_cache` into a
    // `LineLayout`, feeding `measure_document`'s sizing and `paint_lines`/
    // `offset_for_position`/`register_mouse_handling`'s viewport-bounded
    // windowing below.
    let layout = analyze_line_layout(
        &document_cache,
        &row_height_cache,
        &Inputs {
            text,
            cursor,
            focused,
            width: available_width,
            font: &font,
            font_size,
            text_color,
        },
        line_height,
        window,
    );
    let (total_height, cursor_top, cursor_height) = measure_document(&layout, cursor, line_height);
    let render_us = crate::debug_server::frames::micros_since(render_start);

    let editor_div = div()
        .id("editor")
        .test_support()
        .w_full()
        .h(total_height)
        // At least as tall as the scroll pane, so a click anywhere below the
        // text still lands in the editor (and puts the cursor at the end).
        .min_h_full()
        .track_focus(&focus_handle)
        // Must out-rank `gpui_kit::init`'s global `Root` component, which
        // binds plain "tab" to its own focus-next action in a "Root"
        // context wrapping the whole app — see `Editor::handle_insert_tab`'s
        // doc comment for why an `on_key_down` match arm alone can't do this.
        .key_context("TextEditor")
        .on_action(cx.listener(Editor::handle_insert_tab))
        .on_action(cx.listener(Editor::handle_back_tab))
        .on_action(table_action!(cx, TableRowAbove, Op::RowAbove))
        .on_action(table_action!(cx, TableRowBelow, Op::RowBelow))
        .on_action(table_action!(cx, TableColumnLeft, Op::ColumnLeft))
        .on_action(table_action!(cx, TableColumnRight, Op::ColumnRight))
        .on_action(table_action!(cx, TableDeleteRow, Op::DeleteRow))
        .on_action(table_action!(cx, TableDeleteColumn, Op::DeleteColumn))
        .on_action(table_action!(cx, TableAlignLeft, Op::Align(Align::Left)))
        .on_action(table_action!(
            cx,
            TableAlignCenter,
            Op::Align(Align::Center)
        ))
        .on_action(table_action!(cx, TableAlignRight, Op::Align(Align::Right)))
        .on_key_down(cx.listener(Editor::handle_key_down))
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, cx| {
                    let mut bounds = bounds;
                    let frame_clock = crate::debug_server::frames::start();
                    let focused = focus_handle.is_focused(window);
                    // Only snap the view back to the cursor the first time
                    // we see this particular cursor position, not on every
                    // repaint — a plain mouse-wheel scroll alone triggers a
                    // repaint too (with the cursor unchanged), and without
                    // this check it would get fought right back to the
                    // cursor's line the moment that line scrolled out of
                    // view. See `last_autoscrolled_cursor`'s own doc comment.
                    if focused && last_autoscrolled_cursor.get() != Some(cursor) {
                        // A caret in a region is scrolled to by where it is
                        // drawn: the region's own top may be screens away (a
                        // float's text makes a region as tall as the text).
                        let in_region = {
                            let editor = editor_entity.read(cx);
                            let doc = editor.document_cache.borrow();
                            caret_in_column(
                                editor.buffer().text(),
                                &doc,
                                &layout,
                                (cursor, cursor, focused),
                                (&font, font_size, line_height),
                                (text_color.into(), bounds),
                                window,
                            )
                        };
                        let (top, height) = in_region.map_or((cursor_top, cursor_height), |r| {
                            (r.origin.y - bounds.origin.y, r.size.height)
                        });
                        autoscroll_to_cursor(&scroll_handle, bounds, top, height);
                        last_autoscrolled_cursor.set(Some(cursor));
                    }
                    let viewport = scroll_handle.bounds();
                    // `render` sized the rows from the previous layout's width, so a
                    // sidebar toggle reaches it one frame late. Layout is final here:
                    // rebuild for the real width before anything is placed from the
                    // rows, and `notify` so the next render sizes the scroll extent.
                    let real_width = viewport.size.width;
                    if real_width > px(0.) {
                        let (rebuilt, shift) = editor_entity.update(cx, |editor, _| {
                            let inp = Inputs {
                                text: editor.buffer().text(),
                                cursor,
                                focused,
                                width: real_width,
                                font: &font,
                                font_size,
                                text_color,
                            };
                            let rebuilt = layout.update_rows(&inp, window);
                            // Making the lines in view exact can move the content
                            // (rows corrected above it): keep it where the reader
                            // had it, for this frame and the scroll offset.
                            let shift = if viewport.size.height > px(0.) {
                                layout.verify_anchored((bounds, viewport), &inp, window)
                            } else {
                                px(0.)
                            };
                            (rebuilt, shift)
                        });
                        if shift != px(0.) {
                            bounds.origin.y -= shift;
                            let mut offset = scroll_handle.offset();
                            offset.y =
                                (offset.y - shift).clamp(-scroll_handle.max_offset().y, px(0.));
                            scroll_handle.set_offset(offset);
                        }
                        if rebuilt {
                            // A `notify` made while painting is dropped; defer it.
                            let entity = editor_entity.clone();
                            window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                        }
                    }
                    let view = Rc::new(FrameView {
                        layout: layout.clone(),
                        cursor,
                        focused,
                        bounds,
                        viewport,
                        line_height,
                        font: font.clone(),
                        font_size,
                        text_color,
                        table_hover: editor_entity.read(cx).table_hover.clone(),
                        table_drag: editor_entity.read(cx).table_drag,
                        column_hover: editor_entity.read(cx).column_hover,
                        column_drag: editor_entity.read(cx).column_drag.clone(),
                        hover_link: editor_entity.read(cx).hover_link.clone(),
                        float_spots: editor_entity.read(cx).float_spots.clone(),
                    });
                    editor_entity.update(cx, |editor, cx| {
                        editor.last_view = Some(view.clone());
                        if editor.hovering_link {
                            window.set_window_cursor_style(CursorStyle::PointingHand);
                        }
                        let doc = editor.document_cache.borrow();
                        paint_lines(
                            editor.buffer().text(),
                            &editor.link_table,
                            &view,
                            &doc,
                            selection.clone(),
                            window,
                            cx,
                        );
                    });

                    register_mouse_handling(
                        editor_entity.clone(),
                        view.clone(),
                        focus_handle.clone(),
                        window,
                    );

                    // Only registers while `focus_handle` is actually focused
                    // (checked internally by `handle_input`) — installs this
                    // editor as the platform's text input target so typing
                    // and IME composition reach `input_handler.rs` instead of
                    // (or before falling through to) `.on_key_down`.
                    window.handle_input(
                        &focus_handle,
                        ElementInputHandler::new(bounds, editor_entity.clone()),
                        cx,
                    );
                    crate::debug_server::frames::finish(frame_clock, render_us);
                },
            )
            .size_full(),
        )
        .debug_name("editor");
    // The menu's wrapper is its own element: the named editor element is not a
    // parent element, so it sits inside; both fill the pane.
    div()
        .id("editor-menu")
        .w_full()
        .h(total_height)
        .min_h_full()
        .context_menu(table_menu(menu_editor))
        .child(editor_div)
}

/// The right-click menu: table structure for the caret's cell, else the column
/// and float entries, nothing (so no menu) where neither applies. The caret was moved to the clicked cell just before
/// (`register_mouse_handling`), so the entries are built for that cell.
pub(super) fn table_menu(
    editor: Entity<Editor>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _window, cx| {
        let columns = column_menu(&editor, cx);
        let Some(state) = editor.read(cx).table_menu_state() else {
            return match columns {
                Some(items) => items.into_iter().fold(menu, |menu, item| menu.item(item)),
                None => menu,
            };
        };
        let item = |label: &'static str, op: Op, enabled: bool| {
            let editor = editor.clone();
            PopupMenuItem::new(label)
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    editor.update(cx, |editor, cx| {
                        editor.run_table_op(op, window, cx);
                    });
                })
        };
        let align = |label: &'static str, to: Align| {
            item(label, Op::Align(to), true).checked(state.align == to)
        };
        let column_items = columns.unwrap_or_else(|| {
            vec![
                PopupMenuItem::new("Make columns").disabled(true),
                PopupMenuItem::new("Unwrap columns").disabled(true),
            ]
        });
        let menu = menu
            .item(item("Insert row above", Op::RowAbove, state.can_edit_row))
            .item(item("Insert row below", Op::RowBelow, true))
            .item(item("Insert column left", Op::ColumnLeft, true))
            .item(item("Insert column right", Op::ColumnRight, true))
            .separator()
            .item(item("Delete row", Op::DeleteRow, state.can_edit_row))
            .item(item(
                "Delete column",
                Op::DeleteColumn,
                state.can_delete_column,
            ))
            .separator()
            .item(align("Align left", Align::Left))
            .item(align("Align center", Align::Center))
            .item(align("Align right", Align::Right))
            .separator();
        column_items
            .into_iter()
            .fold(menu, |menu, item| menu.item(item))
    }
}

/// The column entries for the caret: add a column and unwrap inside a
/// region, flip and unwrap on a float, "Make columns" and float entries on a
/// block, inserting an empty one on a blank line, none elsewhere (code,
/// callouts, headings: a marker would cut them).
pub(super) fn column_menu(editor: &Entity<Editor>, cx: &App) -> Option<Vec<PopupMenuItem>> {
    let state = editor.read(cx).column_menu_state()?;
    let run = |label: &'static str,
               enabled: bool,
               action: fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool| {
        let editor = editor.clone();
        PopupMenuItem::new(label)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    action(editor, window, cx);
                });
            })
    };
    Some(match state {
        ColumnMenu::Blank => vec![
            run("Insert columns", true, Editor::insert_columns),
            run("Insert float right", true, |e, w, c| {
                e.insert_float(true, w, c)
            }),
            run("Insert float left", true, |e, w, c| {
                e.insert_float(false, w, c)
            }),
        ],
        ColumnMenu::Block => vec![
            run("Make columns", true, Editor::make_columns),
            run("Float right", true, |e, w, c| e.make_float(true, w, c)),
            run("Float left", true, |e, w, c| e.make_float(false, w, c)),
        ],
        ColumnMenu::Aside => vec![
            run("Flip side", true, Editor::flip_aside),
            run("Unwrap float", true, |e, w, c| {
                e.run_region_menu(false, w, c)
            }),
        ],
        ColumnMenu::Float => vec![
            run("Flip side", true, Editor::flip_float),
            run("Move float up", true, |e, w, c| e.move_float(false, w, c)),
            run("Move float down", true, |e, w, c| e.move_float(true, w, c)),
            run("Unwrap float", true, Editor::unwrap_float),
        ],
        ColumnMenu::Region { can_add } => vec![
            run("Add column", can_add, |e, w, c| {
                e.run_region_menu(true, w, c)
            }),
            run("Unwrap columns", true, |e, w, c| {
                e.run_region_menu(false, w, c)
            }),
        ],
    })
}
