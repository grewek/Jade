//! Pointer input: hit-testing a click to a buffer offset and the mouse listeners
//! registered each frame, plus the pointer helpers for tables and regions.

use super::*;
#[allow(unused_imports)]
use crate::text_editor::table;

/// Registers this frame's mouse listeners: press to focus and place the
/// cursor (collapsing any selection), drag (move while the button that
/// pressed inside these bounds is still down) to extend it, release to stop
/// dragging. `window.on_mouse_event` listeners last one frame each — this
/// runs on every paint, same as the painting itself.
pub(super) fn register_mouse_handling(
    editor_entity: Entity<Editor>,
    view: Rc<FrameView>,
    focus_handle: FocusHandle,
    window: &mut Window,
) {
    let bounds = view.bounds;

    // A press on a region's divider grip starts a ratio drag; `+` adds a
    // column and the cross unwraps. Capture phase, like the table controls.
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let view = view.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture
                || event.button != MouseButton::Left
                || window.has_active_dialog(cx)
            {
                return;
            }
            if let Some((region, percent)) =
                restore_pill_at(editor_entity.read(cx), &view, event.position)
            {
                cx.stop_propagation();
                editor_entity.update(cx, |editor, cx| {
                    editor.set_region_ratio(region, percent, window, cx);
                });
                return;
            }
            let Some((region, handle)) =
                column_handle_for_pointer(editor_entity.read(cx), &view, event.position)
            else {
                return;
            };
            cx.stop_propagation();
            editor_entity.update(cx, |editor, cx| {
                editor.column_hover = None;
                match handle {
                    columns::handles::Handle::Add => {
                        editor.add_region_column(region, window, cx);
                    }
                    columns::handles::Handle::Unwrap => {
                        editor.unwrap_region(region, window, cx);
                    }
                    columns::handles::Handle::Divider(divider) => {
                        let percent = {
                            let doc = editor.document_cache.borrow();
                            let r = doc.columns()[region];
                            r.percent[..r.count].to_vec()
                        };
                        editor.column_drag = Some(ColumnDrag {
                            region,
                            divider,
                            percent,
                        });
                        cx.notify();
                    }
                }
            });
        }
    });
    // A press on a table's `+` adds the row or column there, on a grip starts
    // dragging it. In the capture phase, and stopping the event, since the
    // controls straddle the table's edge, partly outside `bounds`.
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let view = view.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture
                || event.button != MouseButton::Left
                || window.has_active_dialog(cx)
            {
                return;
            }
            let Some((table, handle)) =
                handle_for_pointer(editor_entity.read(cx), &view, event.position, window)
            else {
                return;
            };
            cx.stop_propagation();
            editor_entity.update(cx, |editor, cx| match handle {
                Handle::AddColumn(_) | Handle::AddRow(_) => {
                    editor.add_at_handle(table, handle, window, cx);
                }
                Handle::ColumnGrip(at) | Handle::RowGrip(at) => {
                    editor.table_drag = Some(TableDrag {
                        table,
                        grip: handle,
                        over: at,
                    });
                    cx.notify();
                }
            });
        }
    });

    // A right-click in a table puts the caret in the clicked cell before the
    // context menu (which reads it) opens: capture runs ahead of the menu's
    // own bubble listener.
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let view = view.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture
                || event.button != MouseButton::Right
                || !bounds.contains(&event.position)
                || window.has_active_dialog(cx)
            {
                return;
            }
            let offset =
                hit_for_position(editor_entity.read(cx), &view, event.position, window).offset;
            editor_entity.update(cx, |editor, cx| {
                if editor.is_table_line_at(offset) || editor.buffer().selection().is_none() {
                    editor.buffer_mut().set_cursor(offset, false);
                    cx.notify();
                }
            });
        }
    });
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let view = view.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
                // A raw `window.on_mouse_event` listener isn't hitbox-aware
                // — it fires purely off `bounds.contains(...)`, regardless
                // of a `Dialog`'s own occluding backdrop sitting visually
                // on top (GPUI's `occlude()` only gates `Hitbox::is_hovered`-
                // based convenience wrappers like `.on_click`, never the raw
                // mouse-listener dispatch loop). Without this guard, a click
                // on a dialog that happens to be positioned over the editor
                // (e.g. Settings) also starts a drag-select here.
                || window.has_active_dialog(cx)
            {
                return;
            }
            let Hit {
                offset,
                checkbox,
                fold,
            } = hit_for_position(editor_entity.read(cx), &view, event.position, window);
            // A click on a task's checkbox glyph toggles it, and on a foldable
            // callout's rendered header folds it; the cursor stays put.
            if let Some(checkbox) = checkbox {
                editor_entity.update(cx, |editor, cx| {
                    editor.toggle_checkbox(checkbox, window, cx)
                });
                return;
            }
            if let Some(callout) = fold {
                editor_entity.update(cx, |editor, cx| editor.toggle_fold(callout, cx));
                return;
            }
            // On a rendered line a plain click follows a link; on the cursor's
            // own (raw) line only Ctrl/Cmd+click does, so the text stays editable.
            let follow = {
                let editor = editor_entity.read(cx);
                (event.modifiers.secondary() || !editor.on_cursor_line(offset))
                    .then(|| editor.link_event_at(offset))
                    .flatten()
            };
            if let Some(follow) = follow {
                editor_entity.update(cx, |_, cx| cx.emit(follow));
                return;
            }
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
            if phase != DispatchPhase::Bubble {
                return;
            }
            let dialog = window.has_active_dialog(cx);
            if let Some(drag) = editor_entity.read(cx).column_drag.clone() {
                let percent = column_drag_percent(
                    editor_entity.read(cx),
                    &view,
                    &drag,
                    event.position,
                    window,
                );
                if let Some(percent) = percent.filter(|p| !dialog && *p != drag.percent) {
                    editor_entity.update(cx, |editor, cx| {
                        editor.column_drag = Some(ColumnDrag { percent, ..drag });
                        cx.notify();
                    });
                }
                return;
            }
            if let Some(drag) = editor_entity.read(cx).table_drag {
                let over =
                    drop_for_pointer(editor_entity.read(cx), &view, drag, event.position, window);
                if let Some(over) = over.filter(|&o| !dialog && o != drag.over) {
                    editor_entity.update(cx, |editor, cx| {
                        editor.table_drag = Some(TableDrag { over, ..drag });
                        cx.notify();
                    });
                }
                return;
            }
            if !editor_entity.read(cx).is_dragging {
                // The table controls follow the pointer over a table.
                let hover = if dialog {
                    None
                } else {
                    hover_for_pointer(editor_entity.read(cx), &view, event.position, window)
                };
                let over = (!dialog)
                    .then(|| {
                        column_hover_for_pointer(editor_entity.read(cx), &view, event.position)
                    })
                    .flatten();
                if editor_entity.read(cx).column_hover != over {
                    editor_entity.update(cx, |editor, cx| {
                        editor.column_hover = over;
                        cx.notify();
                    });
                }
                if editor_entity.read(cx).table_hover != hover {
                    editor_entity.update(cx, |editor, cx| {
                        editor.table_hover = hover;
                        cx.notify();
                    });
                }
                // Not selecting: only the pointer cursor over a followable
                // link needs working out. Shaping lines to find the offset
                // is skipped for notes without links and outside the editor.
                let probe = !dialog
                    && bounds.contains(&event.position)
                    && editor_entity.read(cx).link_count() > 0;
                let hovering = probe && {
                    let editor = editor_entity.read(cx);
                    let offset = offset_for_position(editor, &view, event.position, window);
                    editor.follows_link_at(offset, event.modifiers.secondary())
                };
                if editor_entity.read(cx).hovering_link != hovering {
                    editor_entity.update(cx, |editor, cx| {
                        editor.hovering_link = hovering;
                        cx.notify();
                    });
                }
                return;
            }
            if dialog {
                return;
            }
            let offset = offset_for_position(editor_entity.read(cx), &view, event.position, window);
            editor_entity.update(cx, |editor, cx| {
                let offset = editor.clamp_to_cell(offset);
                editor.buffer_mut().set_cursor(offset, true);
                cx.notify();
            });
        }
    });

    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        editor_entity.update(cx, |editor, cx| {
            if editor.table_drag.is_some() {
                editor.finish_table_drag(window, cx);
            }
            if editor.column_drag.is_some() {
                editor.finish_column_drag(window, cx);
            }
            if editor.is_dragging {
                editor.is_dragging = false;
                cx.notify();
            }
        });
    });
}

/// The byte offset closest to `position` (window coordinates): the line
/// whose accumulated vertical extent `position`'s `y` falls into (no
/// longer a plain division by `line_height`, now that a wrapped line's own
/// height can be a multiple of it — see `shape_display_line`'s
/// `wrap_width_for`), then the closest char boundary within it — in its
/// *display* text, i.e. accounting for any Live Preview markup hidden on a
/// non-active line — via `WrappedLineLayout::closest_index_for_position`
/// (through `WrappedLine`'s `Deref`, now taking a full `Point` rather than
/// a bare `x`, since which *row* of a wrapped line matters too), mapped
/// back to a raw buffer offset via `DisplayLine::raw_offset_for_display`.
/// Only reshapes lines within `visible_line_range` (a click outside that —
/// shouldn't happen in practice, since clicks land inside the visible
/// viewport by construction — falls back to that range's own nearest
/// edge rather than scanning the rest of the document).
pub(super) fn offset_for_position(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> usize {
    hit_for_position(editor, view, position, window).offset
}

/// What a click at a point lands on.
pub(super) struct Hit {
    pub(super) offset: usize,
    /// The raw range of the task checkbox (`[ ]`/`[x]`) when on its glyph.
    pub(super) checkbox: Option<Range<usize>>,
    /// The foldable callout whose rendered header is under the point.
    pub(super) fold: Option<usize>,
}

impl Hit {
    pub(super) fn at(offset: usize) -> Self {
        Self {
            offset,
            checkbox: None,
            fold: None,
        }
    }
}

/// [`offset_for_position`], plus what else is under the point.
pub(super) fn hit_for_position(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> Hit {
    let text = editor.buffer().text();
    let doc = editor.document_cache.borrow();
    let &FrameView {
        ref layout,
        cursor,
        focused,
        bounds,
        viewport,
        line_height,
        ref font,
        font_size,
        text_color,
        ..
    } = view;
    let relative_y = (position.y - bounds.origin.y).max(px(0.));
    let range = visible_line_range(layout, bounds, viewport);
    if range.is_empty() {
        return Hit::at(0);
    }

    let frame = region::HitFrame {
        bounds,
        relative_y,
        position,
        line_height,
        font,
        font_size,
        text_color,
    };
    let mut y_offset = layout.cumulative_at(range.start);
    for i in range.clone() {
        if let Some((region, split)) = doc.side_by_side(i)
            && i == region.start_line
        {
            let shaped = region::shape_region(
                text,
                &doc,
                (&region, &split),
                (cursor, focused),
                (font, font_size, line_height),
                (text_color.into(), None),
                window,
            );
            let height = line_height * shaped.rows as f32;
            if relative_y < y_offset + height {
                let place = (y_offset, (cursor, focused));
                return region::hit(&doc, text, &shaped, &region, place, &frame, window);
            }
            y_offset += height;
            continue;
        }
        let line_range = layout.line_range(i);
        let line_text = &text[line_range.clone()];
        let state = LineState::of(&doc, i, cursor, focused);
        let results = shape_display_line(
            line_text,
            &state,
            font,
            font_size,
            text_color,
            bounds.size.width,
            window,
        );
        let line = (i, line_text, line_range.start);
        match hit_in_line(&doc, line, (&state, &results), y_offset, &frame, window) {
            Ok(hit) => return hit,
            Err(height) => y_offset += height,
        }
    }
    // Past the visible window's last line (clicked below all rendered
    // text): land at the end of that window's last line, not the
    // document's — the window already covers the whole viewport plus
    // overscan, so this is only reachable by clicking well past the
    // rendered content.
    let offset = if range.end >= layout.line_count() {
        text.len()
    } else {
        layout.line_range(range.end - 1).end
    };
    Hit::at(offset)
}

/// Where a click at `frame.position` lands in one shaped line, whose top is
/// `y_offset` below the editor's top; `Err` is the line's height when the click
/// is below it (the caller moves on to the next line).
pub(super) fn hit_in_line(
    doc: &DocumentCache,
    (i, line_text, line_start): (usize, &str, usize),
    (state, results): (&LineState, &[(WrappedLine, DisplayLine)]),
    y_offset: Pixels,
    frame: &region::HitFrame,
    window: &mut Window,
) -> Result<Hit, Pixels> {
    let &region::HitFrame {
        bounds,
        relative_y,
        position,
        line_height,
        font,
        font_size,
        text_color,
    } = frame;
    // Usually one entry; more than one only for a table row whose cells had to
    // wrap, each entry then its own (unwrapped, since already sized to fit)
    // sub-row of that row.
    let multi_entry = results.len() > 1;
    if let (false, TableLineKind::Row(table)) = (state.table_active, &state.table_kind) {
        let height: Pixels = results
            .iter()
            .map(|(wrapped, _)| wrapped.size(line_height).height)
            .sum();
        if relative_y >= y_offset + height {
            return Err(height);
        }
        let columns = TableColumns::of(
            table,
            bounds.size.width,
            (font, font_size, text_color),
            window,
        );
        let row = ((relative_y - y_offset) / line_height) as usize;
        let x = (position.x - bounds.origin.x) / columns.char_width;
        let offset = table::offset_at_editing(
            line_text,
            &columns.widths,
            &columns.aligns,
            x,
            row,
            state.editing_cell,
        );
        return Ok(Hit::at(line_start + offset));
    }
    let mut y = y_offset;
    for (wrapped, display_line) in results {
        let line_height = tall_row_height(
            results.len(),
            (display_line, line_text),
            wrapped,
            (font_size, line_height),
        );
        let height = wrapped.size(line_height).height;
        if relative_y < y + height {
            let relative_x =
                (position.x - bounds.origin.x - indent_px(display_line.indent.level())).max(px(0.));
            let local_y = (relative_y - y).max(px(0.));
            let row = (local_y / line_height) as usize;
            let relative_x = justify::unshift_x(
                (state, wrapped, display_line),
                results.len(),
                (row, relative_x),
            );
            let display_offset =
                match wrapped.closest_index_for_position(point(relative_x, local_y), line_height) {
                    Ok(ix) | Err(ix) => ix,
                };
            // A wrapped table sub-row has no genuinely visible segment to land
            // on (every cell's text is `is_replaced()` there, see
            // `build_table_row`'s own doc comment): only
            // `nearest_segment_raw_start` handles that case correctly.
            let raw_offset = if multi_entry {
                display_line.nearest_segment_raw_start(display_offset)
            } else {
                display_line.raw_offset_for_display(display_offset)
            };
            let checkbox = display_line
                .checkbox_at(display_offset)
                .map(|r| line_start + r.start..line_start + r.end);
            let fold = (state.callout_is_header && !state.active)
                .then(|| callout_index_at(doc.callouts(), i))
                .flatten()
                .filter(|&c| doc.callouts()[c].foldable);
            return Ok(Hit {
                offset: line_start + raw_offset,
                checkbox,
                fold,
            });
        }
        y += height;
    }
    Err(y - y_offset)
}

/// The side-by-side region under the pointer (or just above it, where its
/// buttons are): its index, its controls' frame and the pointer in it.
pub(super) fn column_frame_at(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
) -> Option<(usize, columns::handles::Frame, (f32, f32))> {
    let doc = editor.document_cache.borrow();
    let regions = doc.columns();
    let y = position.y - view.bounds.origin.y;
    let margin = px(columns::handles::MARGIN);
    let index = regions
        .partition_point(|r| view.layout.cumulative_at(r.start_line) - margin <= y)
        .checked_sub(1)?;
    let region = regions[index];
    let (_, split) = doc.side_by_side(region.start_line)?;
    let top = view.layout.cumulative_at(region.start_line);
    let height = view.layout.height(region.start_line);
    let frame = columns::handles::Frame {
        dividers: (0..split.divider_count())
            .map(|k| split.divider_x(k))
            .collect(),
        width: split.total,
        height: f32::from(height),
        can_add: region.count < columns::MAX_COLUMNS,
        buttons: !region.tail,
        grips: editor
            .float_spots
            .borrow()
            .iter()
            .find(|(line, _)| *line == region.start_line)
            .map_or_else(Vec::new, |(_, spots)| region::float_grips(spots, &split)),
    };
    let p = (
        f32::from(position.x - view.bounds.origin.x),
        f32::from(y - top),
    );
    Some((index, frame, p))
}

/// The region whose controls show for the pointer.
pub(super) fn column_hover_for_pointer(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
) -> Option<usize> {
    let (index, frame, p) = column_frame_at(editor, view, position)?;
    columns::handles::hover_at(&frame, p).then_some(index)
}

/// The control under the pointer, when its region's controls are shown.
pub(super) fn column_handle_for_pointer(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
) -> Option<(usize, columns::handles::Handle)> {
    let (index, frame, p) = column_frame_at(editor, view, position)?;
    let handle = columns::handles::handle_at(&frame, p)?;
    Some((index, handle))
}

/// The shares dragging `drag`'s divider to the pointer would give.
pub(super) fn column_drag_percent(
    editor: &Editor,
    view: &FrameView,
    drag: &ColumnDrag,
    position: Point<Pixels>,
    window: &mut Window,
) -> Option<Vec<u8>> {
    let doc = editor.document_cache.borrow();
    let region = *doc.columns().get(drag.region)?;
    // From the shares the file has, not the layout the drag previews: that one
    // may be stacked, and the pointer must still be able to bring it back.
    let total = doc.view.column_width()?;
    let k = drag.divider;
    let x = f32::from(position.x - view.bounds.origin.x);
    if region.kind == columns::Kind::Aside {
        // Divider `k` is the inner edge of a side's floats: the pointer is half
        // a gap from it. Every float on that side takes the share, snapped and
        // kept within bounds.
        let floats = region.floats();
        let sides = usize::from(floats.iter().any(|f| !f.right))
            + usize::from(floats.iter().any(|f| f.right));
        let usable = total - columns::GAP * sides as f32;
        let right_side =
            floats.iter().any(|f| !f.right) as usize <= k && floats.iter().any(|f| f.right);
        let float = if right_side {
            total - x - columns::GAP / 2.
        } else {
            x - columns::GAP / 2.
        };
        // In a narrow window the layout scales both sides down to leave the text
        // its channel, so the pointer is on a *drawn* width. Undo that scaling
        // (drawn = room * a / (a + b), b the other side's width) to get the
        // share that draws where the pointer is.
        let shares_on = |right: bool| {
            floats
                .iter()
                .zip(&region.percent)
                .filter(|(f, _)| f.right == right)
                .map(|(_, &share)| share)
                .max()
        };
        let other = shares_on(!right_side).map_or(0., |sh| usable * f32::from(sh) / 100.);
        let room = usable - columns::MIN_CHANNEL;
        let float = if room > 0. && float + other > room && other > 0. {
            let drawn = float.min(room * 0.99);
            drawn * other / (room - drawn)
        } else {
            float
        };
        let step = f32::from(columns::ops::SNAP);
        let share = (float / usable * 100. / step).round() * step;
        let share = share.clamp(f32::from(columns::ASIDE_MIN), f32::from(columns::ASIDE_MAX)) as u8;
        let mut shares = region.percent[..region.count].to_vec();
        for (f, s) in floats.iter().zip(shares.iter_mut()) {
            if f.right == right_side {
                *s = share;
            }
        }
        return Some(shares);
    }
    // Column `k`'s right edge is half a gap left of the pointer.
    let (left, usable) = columns::column_left_unchecked(total, &region.percent[..region.count], k);
    let width = x - columns::GAP / 2. - left;
    let char_width = f32::from(mono_char_width(
        &view.font,
        view.font_size,
        view.text_color,
        window,
    ));
    // A table box cannot shrink below its columns' minimum: keep each side at
    // least that wide so it never paints into its neighbour.
    let share_for = |column: usize| {
        let px = min_column_px(&doc, &region, column, char_width);
        // Shares snap in steps, so round the limit up to one.
        let step = f32::from(columns::ops::SNAP);
        (px / usable * 100. / step).ceil() * step
    };
    let pair = f32::from(region.percent[k] + region.percent[k + 1]);
    let (lo, hi) = (share_for(k), pair - share_for(k + 1));
    let share = width / usable * 100.;
    let share = if lo <= hi { share.clamp(lo, hi) } else { share };
    Some(columns::ops::dragged(
        &region.percent[..region.count],
        k,
        share,
    ))
}

/// The least width, in pixels, column `column` of `region` can have: that of its
/// widest table at the minimum column width (0 with no table).
pub(super) fn min_column_px(
    doc: &DocumentCache,
    region: &Columns,
    column: usize,
    char_width: f32,
) -> f32 {
    let lines = region.column_lines(column);
    let tables = doc.tables();
    let first = tables.partition_point(|t| t.start_line < lines.start);
    tables[first..]
        .iter()
        .take_while(|t| t.start_line < lines.end)
        .map(|t| {
            let widths = vec![table::MIN_COLUMN_WIDTH; t.columns.len()];
            table::box_width(&widths) * char_width
        })
        .fold(0., f32::max)
}

/// The rendered table the pointer is over (or in the margin left of or above
/// it), its grid, and the pointer relative to the table's top left.
pub(super) fn table_at_pointer(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> Option<(usize, Grid, (f32, f32))> {
    let doc = editor.document_cache.borrow();
    let text = editor.buffer().text();
    let tables = doc.tables();
    let y = position.y - view.bounds.origin.y;
    let margin = px(table::handles::MARGIN);
    let relative = |origin: Point<Pixels>| {
        (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    };
    // A side-by-side region lays its tables out inside its columns, where the
    // row cache's line tops do not apply: look at its own tables.
    let regions = doc.columns();
    let at_region = regions
        .partition_point(|r| view.layout.cumulative_at(r.start_line) - margin <= y)
        .checked_sub(1)
        .map(|i| &regions[i])
        .filter(|r| {
            let top = view.layout.cumulative_at(r.start_line);
            doc.side_by_side(r.start_line).is_some() && y <= top + view.layout.height(r.start_line)
        });
    if let Some(region) = at_region {
        let first = tables.partition_point(|t| t.start_line < region.start_line);
        for (k, table) in tables[first..]
            .iter()
            .take_while(|t| t.start_line < region.end_line)
            .enumerate()
        {
            if is_table_active(&doc, table.start_line, view.focused) {
                continue;
            }
            let Some((grid, origin)) = table_grid(view, &doc, text, table, window) else {
                continue;
            };
            let p = relative(origin);
            let (width, height) = (grid.col_x.last().copied(), grid.row_y.last().copied());
            let reach = table::handles::MARGIN;
            let inside = p.0 >= -reach
                && p.1 >= -reach
                && width.is_some_and(|w| p.0 <= w + reach)
                && height.is_some_and(|h| p.1 <= h + reach);
            if inside {
                return Some((first + k, grid, p));
            }
        }
        return None;
    }
    let index = tables
        .partition_point(|t| view.layout.cumulative_at(t.start_line) - margin <= y)
        .checked_sub(1)?;
    let table = &tables[index];
    if is_table_active(&doc, table.start_line, view.focused)
        || doc.side_by_side(table.start_line).is_some()
    {
        return None;
    }
    let (grid, origin) = table_grid(view, &doc, text, table, window)?;
    let p = relative(origin);
    Some((index, grid, p))
}

/// The controls to show for the pointer at `position`.
pub(super) fn hover_for_pointer(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> Option<(usize, Vec<Handle>)> {
    let (index, grid, p) = table_at_pointer(editor, view, position, window)?;
    let shown = table::handles::hover_at(&grid, p);
    (!shown.is_empty()).then_some((index, shown))
}

/// The table control under `position`, if one is shown there.
pub(super) fn handle_for_pointer(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> Option<(usize, Handle)> {
    let (index, grid, p) = table_at_pointer(editor, view, position, window)?;
    let shown = table::handles::hover_at(&grid, p);
    table::handles::handle_at(&grid, &shown, p).map(|handle| (index, handle))
}

/// The drop boundary for a grip drag with the pointer at `position`.
pub(super) fn drop_for_pointer(
    editor: &Editor,
    view: &FrameView,
    drag: TableDrag,
    position: Point<Pixels>,
    window: &mut Window,
) -> Option<usize> {
    let doc = editor.document_cache.borrow();
    let table = doc.tables().get(drag.table)?;
    let (grid, origin) = table_grid(view, &doc, editor.buffer().text(), table, window)?;
    Some(match drag.grip {
        Handle::ColumnGrip(_) => {
            table::handles::column_drop(&grid, f32::from(position.x - origin.x))
        }
        _ => table::handles::row_drop(&grid, f32::from(position.y - origin.y)),
    })
}

/// The stacked region whose restore button is under `position`, with its shares.
pub(super) fn restore_pill_at(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
) -> Option<(usize, Vec<u8>)> {
    let doc = editor.document_cache.borrow();
    if editor.column_drag.is_some() {
        return None;
    }
    (0..doc.columns().len()).find_map(|index| {
        let (bounds, percent) = restore_pill(view, &doc, index)?;
        bounds.contains(&position).then_some((index, percent))
    })
}
