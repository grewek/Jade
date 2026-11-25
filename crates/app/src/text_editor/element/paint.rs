//! Painting the visible lines: backgrounds and bands first, then each line's
//! text, selection and caret.

#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_lines(
    text: &str,
    links: &LinkTable,
    view: &FrameView,
    doc: &DocumentCache,
    selection: Option<std::ops::Range<usize>>,
    window: &mut Window,
    cx: &mut App,
) {
    let (line_roles, tables, callouts) = (doc.line_roles(), doc.tables(), doc.callouts());
    view.float_spots.borrow_mut().clear();
    let &FrameView {
        ref layout,
        cursor,
        focused: show_cursor,
        bounds,
        viewport,
        line_height,
        ref font,
        font_size,
        text_color,
        ..
    } = view;
    for freed in cx.default_global::<ImageCache>().begin_frame() {
        window.drop_image(freed).ok();
    }
    let typography = crate::text_editor::TextSettings::typography(cx);
    let callout_shape = (
        typography.radius_of(crate::settings::Boxed::Callout),
        typography.callout.bar_width,
    );
    let palette = ThemeEngine::global(cx).palette().clone();
    let code_box_look = typography
        .code_block
        .background
        .then(|| code_box::CodeBoxLook {
            fill: palette.color(Role::CodeBackground),
            border: typography.code_block.border.then(|| {
                (
                    px(typography.shape.border_width),
                    palette.color(Role::Border),
                )
            }),
            radius: typography.radius_of(crate::settings::Boxed::CodeBlock),
            padding: typography.code_block.padding,
            label: typography
                .code_block
                .language_label
                .then(|| palette.color(Role::TextSubtle)),
        });
    let underline_look = {
        use crate::settings::Underline::Never;
        let link = typography.link;
        (link.underline != Never || link.wikilink_underline != Never).then(|| {
            link_underline::UnderlineLook {
                color: palette.color(Role::LinkUnderline),
                links: link.underline,
                wikilinks: link.wikilink_underline,
                char_width: mono_char_width(font, font_size, text_color, window),
            }
        })
    };
    let chip_look = typography
        .inline_code
        .background
        .then(|| code_chip::ChipLook {
            fill: palette.color(Role::CodeBackground),
            padding: typography.inline_code.padding,
            radius: typography.radius_of(crate::settings::Boxed::InlineCode),
            char_width: mono_char_width(font, font_size, text_color, window),
        });
    let inline = InlineColors::from_palette(&palette);
    let rule = palette.color(Role::BorderStrong);
    let selection_fill = palette.with_alpha(Role::Selection, Alpha::Selection);
    let widget_colors = WidgetColors {
        border: rule,
        accent: palette.color(Role::Accent),
        on_accent: palette.color(Role::TextOnAccent),
    };
    let cursor_color = palette.color(Role::Cursor);
    let range = visible_line_range(layout, bounds, viewport);
    let mut trace = PaintTrace::start_if_enabled(|| {
        let (cached_width, verified, late_corrections_before) = layout.trace_state();
        let rect = |b: Bounds<Pixels>| {
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            )
        };
        FrameInfo {
            bounds: rect(bounds),
            viewport: rect(viewport),
            cursor,
            focused: show_cursor,
            range: range.clone(),
            total_height: f32::from(layout.cumulative_at(layout.line_count())),
            cached_width: cached_width.map(f32::from),
            verified,
            late_corrections_before,
        }
    });

    // Every callout's background tint + accent bar, painted *before* any
    // line's text — unlike `TablePaint`'s grid lines (thin enough that
    // painting them after a row's own text is fine), a full-width
    // background fill painted afterward would paint right over and hide
    // that text. `callout_ranges` (from `measure_document`'s own
    // `layout.cumulative_at` lookups) gives each callout's already-known
    // top/bottom extent without needing to have visited its lines at all, so
    // this runs as its own pass, skipping any callout whose range doesn't
    // intersect the viewport at all (cheap, and keeps this pass bounded
    // even for a document with a huge number of callouts).
    layout.verify_rows(
        range.clone(),
        &Inputs {
            text,
            cursor,
            focused: show_cursor,
            width: bounds.size.width,
            font,
            font_size,
            text_color,
        },
        window,
    );
    // Callouts never overlap and are ordered, so the visible ones are a
    // contiguous run.
    let first_visible = callouts.partition_point(|c| c.end_line <= range.start);
    for callout in callouts[first_visible..]
        .iter()
        .take_while(|c| c.start_line < range.end)
    {
        // Past the verified window the heights are estimates; the band is
        // off-screen there anyway, so stop where they begin.
        let (top, bottom) = (
            layout.screen_y(bounds.origin.y, callout.start_line),
            layout.screen_y(bounds.origin.y, callout.end_line.min(range.end)),
        );
        if let Some(trace) = trace.as_mut() {
            trace.band(
                callout.start_line..callout.end_line.min(range.end),
                (f32::from(top), f32::from(bottom)),
                callout.kind,
            );
        }
        let span = (bounds.origin.x, bounds.size.width);
        paint_callout_band(
            callout,
            (top, bottom),
            span,
            (&palette, callout_shape),
            window,
        );
    }

    if let Some(look) = &code_box_look {
        code_box::paint_code_boxes(&range, (view, doc, text), look, (window, cx));
    }

    if range.is_empty() {
        return;
    }

    paint_table_boxes(&range, view, doc, &palette, (window, cx, trace.as_mut()));
    // Lines are placed from the first visible one: its window y comes from
    // `screen_y` and everything after it is a small offset, not a sum of
    // numbers in the millions (see `LineLayout::screen_y`).
    let anchor = layout.screen_y(bounds.origin.y, range.start);
    let mut y_offset = px(0.);
    let bounds = Bounds {
        origin: point(bounds.origin.x, anchor),
        size: bounds.size,
    };
    let mut columns: Option<(usize, TableColumns)> = None;
    let env = PaintEnv {
        text,
        doc,
        selection,
        table_hover: view.table_hover.as_ref(),
        table_drag: view.table_drag,
        column_hover: view.column_hover,
        column_drag: view.column_drag.as_ref(),
        show_cursor,
        cursor,
        line_height,
        font,
        font_size,
        text_color,
        palette: &palette,
        inline,
        rule,
        selection_fill,
        widget_colors,
        cursor_color,
        table_shape: BoxShape {
            radius: typography.radius_of(crate::settings::Boxed::Table),
            border_width: typography.shape.border_width,
        },
        checkbox_radius: typography.task.checkbox_radius,
        image_radius: typography.radius_of(crate::settings::Boxed::Image),
        chip_look,
        underline_look,
        hover_link: view.hover_link.as_ref(),
        callout_shape,
    };

    for i in range.clone() {
        if let Some((region, split)) = doc.side_by_side(i)
            && i == region.start_line
        {
            let shaped = region::shape_region(
                text,
                doc,
                (&region, &split),
                (cursor, show_cursor),
                (font, font_size, line_height),
                (inline, Some(links)),
                window,
            );
            view.float_spots
                .borrow_mut()
                .push((i, shaped.float_spots.clone()));
            region::paint_region(&env, &shaped, &region, (bounds, y_offset), (window, cx));
            // Exact rows from shaping, as the row cache does for any line.
            layout.correct_row_count(i, shaped.rows);
            if let Some(trace) = trace.as_mut() {
                trace.region(region::trace_geometry(&env, &shaped, (i, y_offset), window));
                let height = line_height * shaped.rows as f32;
                let first = layout.line_range(i);
                let line = &text[first];
                trace.line(
                    i,
                    f32::from(anchor + y_offset),
                    shaped.rows,
                    f32::from(height),
                    "columns",
                    line,
                );
            }
            y_offset += line_height * shaped.rows as f32;
            continue;
        }
        let line_range = layout.line_range(i);
        let line_text = &text[line_range.clone()];
        let broken: Vec<Range<usize>> = links.broken_on_line(i).collect();
        let state = LineState {
            broken: &broken,
            ..LineState::of(doc, i, cursor, show_cursor)
        };
        let callout = state.callout;
        let results = shape_display_line(
            line_text,
            &state,
            font,
            font_size,
            inline,
            bounds.size.width,
            window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap (see `shape_display_line`'s own doc comment).
        let entry = entry_heights(&results, line_text, (font_size, line_height), state.macros)
            .next()
            .unwrap_or_else(|| EntryHeight::uniform(line_height));
        let (total_height, rows) =
            shaped_height(&results, line_text, (font_size, line_height), state.macros);
        // Upgrades a cold-rebuild estimate (see `RowCache::rebuild`)
        // to the truth using the real shaping this line's painting already
        // needed — zero extra cost, and idempotent (a no-op once this line
        // has already been corrected, e.g. by an earlier viewing or an
        // edit). `.round()` because `total_height` is always an exact
        // multiple of `line_height` in principle, but float division can
        // land a hair off either side of the actual integer.
        layout.correct_row_count(i, rows);
        if results.is_empty() {
            continue;
        }
        if let Some(trace) = trace.as_mut() {
            let tag = match (callout, table_line_kind(tables, i), line_roles[i]) {
                _ if matches!(state.math, MathPart::Head(_)) => "math",
                _ if image::embed(line_text, &state).is_some() => "image",
                (Some(c), ..) if c.is_header(i) => "callout-header",
                (Some(_), ..) => "callout",
                (None, TableLineKind::Row(_) | TableLineKind::Delimiter, _) => "table",
                (None, _, LineRole::FenceContent) => "code",
                _ => "plain",
            };
            trace.line(
                i,
                f32::from(anchor + y_offset),
                rows,
                f32::from(total_height),
                tag,
                line_text,
            );
            trace.set_late_corrections(layout.trace_state().2);
        }
        let slot = LineSlot {
            i,
            line_text,
            line_range: line_range.clone(),
            state: &state,
            results: &results,
            bounds,
            y_offset,
            total_height,
            entry: &entry,
        };
        paint_line(&env, &slot, &mut columns, window, cx);

        y_offset += total_height;
    }
    let handle_palette = handle_colors(&palette, text_color);
    paint_stacked_controls(view, doc, &handle_palette, (window, cx));
}

/// A callout's background tint and accent bar, from `top` to `bottom`
/// (window y), across `(left, width)`.
pub(super) fn paint_callout_band(
    callout: &Callout,
    (top, bottom): (Pixels, Pixels),
    (left, width): (Pixels, Pixels),
    (palette, (radius, bar_width)): (&crate::theme::Palette, (crate::settings::Radius, f32)),
    window: &mut Window,
) {
    let color = palette.color(callout_role(callout.kind));
    let background = palette.with_alpha(callout_role(callout.kind), Alpha::CalloutBackground);
    let band = Bounds {
        origin: point(left, top),
        size: size(width, bottom - top),
    };
    paint_box(band, radius, background, None, window);
    if bar_width > 0.0 {
        // Rounded, the bar is the box's own left edge; square, it keeps its inset.
        let x0 = if matches!(radius, crate::settings::Radius::Px(r) if r <= 0.0) {
            QUOTE_BAR_INSET
        } else {
            px(0.)
        };
        let x1 = (x0 + px(bar_width)).min(width);
        paint_strip(band, radius, (x0, x1), color, window);
    }
}

/// What painting any line of a frame needs, whichever column it is in.
pub(super) struct PaintEnv<'a> {
    pub(super) text: &'a str,
    pub(super) doc: &'a DocumentCache,
    pub(super) selection: Option<Range<usize>>,
    /// The table controls shown and the grip being dragged.
    pub(super) table_hover: Option<&'a (usize, Vec<Handle>)>,
    pub(super) table_drag: Option<TableDrag>,
    pub(super) column_hover: Option<usize>,
    pub(super) column_drag: Option<&'a ColumnDrag>,
    pub(super) show_cursor: bool,
    pub(super) cursor: usize,
    pub(super) line_height: Pixels,
    pub(super) font: &'a Font,
    pub(super) font_size: Pixels,
    pub(super) text_color: Hsla,
    pub(super) palette: &'a crate::theme::Palette,
    pub(super) inline: InlineColors,
    pub(super) rule: Hsla,
    pub(super) selection_fill: Hsla,
    pub(super) widget_colors: WidgetColors,
    pub(super) cursor_color: Hsla,
    /// Outline and corners of a rendered table's box.
    pub(super) table_shape: BoxShape,
    /// Corners of a task checkbox.
    pub(super) checkbox_radius: crate::settings::Radius,
    /// Corners of an image and of its placeholder.
    pub(super) image_radius: crate::settings::Radius,
    /// The chip behind inline code, when `inline_code.background` is on.
    pub(super) chip_look: Option<code_chip::ChipLook>,
    /// How links are underlined, and the link under the pointer.
    pub(super) underline_look: Option<link_underline::UnderlineLook>,
    pub(super) hover_link: Option<&'a link_underline::HoverLink>,
    /// Corners and accent bar width of a callout's band.
    pub(super) callout_shape: (crate::settings::Radius, f32),
}

impl<'a> PaintEnv<'a> {
    /// The palette and the (corners, bar width) a callout's band is painted with.
    pub(super) fn band_style(&self) -> (&'a crate::theme::Palette, (crate::settings::Radius, f32)) {
        (self.palette, self.callout_shape)
    }
}

/// One shaped line and where it goes: `bounds` is the frame it is painted
/// in (the editor's, or a column's: same top, the column's left and width),
/// `y_offset` its top below `bounds.origin.y`.
pub(super) struct LineSlot<'a> {
    pub(super) i: usize,
    pub(super) line_text: &'a str,
    pub(super) line_range: Range<usize>,
    pub(super) state: &'a LineState<'a>,
    pub(super) results: &'a [(WrappedLine, DisplayLine)],
    pub(super) bounds: Bounds<Pixels>,
    pub(super) y_offset: Pixels,
    pub(super) total_height: Pixels,
    /// The heights of this line's rows: the editor's, or a whole multiple of
    /// it where a tall formula sits (see `entry_heights`).
    pub(super) entry: &'a EntryHeight,
}

/// Paints one shaped line (selection, text, widgets, caret, rules, images,
/// table cells). `columns` is the table whose widths were last worked out,
/// kept across the lines of one frame (or one column).
pub(super) fn paint_line(
    env: &PaintEnv,
    slot: &LineSlot,
    columns: &mut Option<(usize, TableColumns)>,
    window: &mut Window,
    cx: &mut App,
) {
    let PaintEnv {
        doc,
        show_cursor,
        cursor,
        font,
        font_size,
        text_color,
        palette,
        inline,
        rule,
        selection_fill,
        ref widget_colors,
        cursor_color,
        checkbox_radius,
        image_radius,
        ref chip_look,
        ref underline_look,
        hover_link,
        ..
    } = *env;
    let selection = env.selection.clone();
    let (line_roles, tables) = (doc.line_roles(), doc.tables());
    let LineSlot {
        i,
        line_text,
        ref line_range,
        state,
        results,
        bounds,
        y_offset,
        total_height,
        entry,
    } = *slot;
    let line_height = entry.row;
    let line_range = line_range.clone();
    let callout = state.callout;

    // Every entry of one raw line shares the same indent (a table's is
    // always `None` across all of them, same as the common one-entry
    // case) — fine to read it off the first.
    let indent = results[0].1.indent;
    let line_origin = bounds.origin + point(indent_px(indent), y_offset);
    // The width a wrapped row should be highlighted to when a
    // selection continues past it onto the next row — the same budget
    // `wrap_width_for` gave this line to wrap within.
    let full_row_width = bounds.size.width - indent_px(indent);

    // A thematic break (`---`/`***`/`___`) renders as a real vector
    // line, same "quad over a blank `hidden_line` gap" technique a
    // table's own grid lines use — a monospace rule glyph repeated
    // across the line wouldn't reliably abut pixel-for-pixel either.
    // Only for a `LineRole::Plain` line outside any table (matching
    // `shape_display_line`'s own gating) and not the active line (which
    // renders the raw `---` text instead, same as every other construct).
    // `table_kind` was already moved into `shape_display_line` above —
    // cheap to resolve again rather than restructuring this loop just
    // to keep the first value alive (same reasoning as
    // `table_kind_for_geometry` below).
    if !state.active
        && matches!(line_roles[i], LineRole::Plain)
        && matches!(table_line_kind(tables, i), TableLineKind::None)
        && is_thematic_break(line_text)
    {
        let mid_y = bounds.origin.y + y_offset + total_height / 2.;
        window.paint_quad(fill(
            Bounds {
                origin: point(bounds.origin.x, mid_y),
                size: size(bounds.size.width, TABLE_LINE_WIDTH),
            },
            rule,
        ));
    }

    if let Some(c) = callout.filter(|c| c.foldable && c.is_header(i) && !state.active) {
        let color = palette.color(callout_role(c.kind));
        let icon = if c.folded {
            "icons/chevron-right.svg"
        } else {
            "icons/chevron-down.svg"
        };
        let side = font_size;
        let at = point(
            bounds.origin.x + bounds.size.width - px(28.),
            bounds.origin.y + y_offset + (line_height - side) / 2.,
        );
        paint_icon(
            icon,
            Bounds {
                origin: at,
                size: size(side, side),
            },
            color,
            (window, cx),
        );
    }

    if let (MathPart::Head(_), Some(block)) = (state.math, doc.math_block_at(i)) {
        let area = Bounds {
            origin: point(bounds.origin.x, bounds.origin.y + y_offset),
            size: size(bounds.size.width, total_height),
        };
        let body = doc.math_body(env.text, block);
        paint_math_block(
            body,
            area,
            font_size,
            (inline.text, inline.broken),
            doc.macros(),
            (window, cx),
        );
    }

    if let Some(embed) = image::embed(line_text, &state) {
        let area = Bounds {
            origin: point(bounds.origin.x, bounds.origin.y + y_offset),
            size: size(bounds.size.width, total_height),
        };
        let target = &line_text[embed.target.clone()];
        let alt = &line_text[embed.alt.clone()];
        let wiki = line_text.trim_start().starts_with("![[");
        let drawn = doc.view.images().info(target).is_some_and(|info| {
            let requested = image::requested_width(alt, wiki);
            paint_image(info, requested, area, image_radius, window, cx)
        });
        if !drawn {
            paint_image_placeholder(
                target,
                area,
                (font, font_size),
                (rule, inline.subtle, image_radius),
                (window, cx),
            );
        }
    }

    let rendered_table_row =
        !state.table_active && matches!(state.table_kind, TableLineKind::Row(_));
    if let (true, TableLineKind::Row(table)) = (rendered_table_row, &state.table_kind) {
        let index = table_index_at(tables, i).unwrap_or_default();
        if columns.as_ref().is_none_or(|(n, _)| *n != index) {
            let boxed = TableColumns::of(
                table,
                bounds.size.width,
                (font, font_size, text_color),
                window,
            );
            *columns = Some((index, boxed));
        }
        if let Some((_, columns)) = &columns {
            let scale = table::row_scale(
                line_text,
                &columns.widths,
                (state.editing_cell, state.macros),
                results.len(),
            );
            paint_table_row(
                line_text,
                columns,
                bounds.origin + point(px(0.), y_offset),
                (table.is_first_row(i), state.editing_cell, scale),
                (font, font_size, line_height),
                (&inline, widget_colors, state.macros),
                window,
                cx,
            );
            if let Some(column) = state.editing_cell {
                let cell = EditingCell {
                    line: line_text,
                    line_start: line_range.start,
                    column,
                    origin: bounds.origin + point(px(0.), y_offset),
                    line_height,
                    scale,
                    macros: state.macros,
                };
                let (selection, caret) = (selection.as_ref(), show_cursor.then_some(cursor));
                paint_editing_cell(
                    &cell,
                    columns,
                    (selection, caret),
                    (palette.color(Role::Accent), selection_fill, cursor_color),
                    window,
                );
            }
        }
    }

    // A callout's own accent bar (painted in the pre-pass above, at
    // this same gutter position since a callout is always depth-1)
    // already covers this line — painting the plain muted bar too
    // would paint right over it, hiding the color.
    if callout.is_none()
        && let LineIndent::Blockquote(depth) = results[0].1.indent
    {
        let (unit, bar) = (
            crate::settings::quote_indent_px(),
            crate::settings::quote_bar_width(),
        );
        // The bar sits inside its level's indent, a little in from the edge.
        let inset = px(((unit - bar) / 2.).clamp(0., f32::from(QUOTE_BAR_INSET)));
        for level in 0..depth {
            let bar_x = bounds.origin.x + indent_px(LineIndent::Blockquote(level)) + inset;
            window.paint_quad(fill(
                Bounds {
                    origin: point(bar_x, bounds.origin.y + y_offset),
                    size: size(px(bar), total_height),
                },
                rule,
            ));
        }
    }

    if results.len() > 1 || rendered_table_row {
        // A wrapped table row — see `shape_display_line`'s own doc
        // comment on why this sub-row split can't carry a byte-exact
        // raw↔display mapping the way a regular wrapped paragraph
        // does, which rules out the per-character selection/cursor
        // placement below. Pragmatic fallback: highlight the row's
        // *whole* combined block if the selection touches it at all,
        // and always place the caret on its first sub-row.
        if let Some(selection) = &selection
            && state.editing_cell.is_none()
            && selection.start.max(line_range.start) < selection.end.min(line_range.end)
        {
            window.paint_quad(fill(
                Bounds {
                    origin: line_origin,
                    size: size(full_row_width, total_height),
                },
                selection_fill,
            ));
        }
        let mut row_y = y_offset;
        for (wrapped, _) in results {
            let _ = wrapped.paint(
                bounds.origin + point(indent_px(indent), row_y),
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            row_y += wrapped.size(line_height).height;
        }
        if show_cursor
            && state.editing_cell.is_none()
            && cursor >= line_range.start
            && cursor <= line_range.end
        {
            window.paint_quad(fill(
                Bounds {
                    origin: line_origin,
                    size: size(px(1.), line_height),
                },
                cursor_color,
            ));
        }
    } else {
        let (wrapped, display_line) = &results[0];
        if let Some(look) = underline_look {
            link_underline::paint_link_underlines(
                (state, wrapped, display_line),
                (entry, line_origin),
                (look, i, hover_link),
                window,
            );
        }
        if let Some(look) = &chip_look {
            code_chip::paint_code_chips(
                (state, wrapped, display_line),
                (entry, line_origin),
                look,
                window,
            );
        }

        if let Some(selection) = &selection {
            let start = selection.start.max(line_range.start);
            let end = selection.end.min(line_range.end);
            if start < end {
                let start_pos = justify::position_for_index(
                    (state, wrapped, display_line),
                    results.len(),
                    display_line.display_offset_for_raw(start - line_range.start),
                    line_height,
                )
                .unwrap_or(point(px(0.), px(0.)));
                let end_pos = justify::position_for_index(
                    (state, wrapped, display_line),
                    results.len(),
                    display_line.display_offset_for_raw(end - line_range.start),
                    line_height,
                )
                .unwrap_or(point(
                    full_row_width,
                    total_height - entry.pad.0 - entry.pad.1 - line_height,
                ));
                // Rows are `line_height` apart in the layout, however tall they are painted.
                let row_of = |y: Pixels| (f32::from(y) / f32::from(line_height)).round() as usize;
                let (start_row, end_row) = (row_of(start_pos.y), row_of(end_pos.y));
                let (start_pos, start_height) = entry.place(start_pos);
                let (end_pos, _) = entry.place(end_pos);
                if start_row == end_row {
                    window.paint_quad(fill(
                        Bounds {
                            origin: line_origin + start_pos,
                            size: size(end_pos.x - start_pos.x, start_height),
                        },
                        selection_fill,
                    ));
                } else {
                    // Spans more than one wrapped row of this same
                    // line: the first row highlights from the
                    // selection's start to the row's own right edge,
                    // the last row from the row's left edge to the
                    // selection's end, and any rows in between are
                    // highlighted in full.
                    for row in start_row..=end_row {
                        let (x0, x1) = if row == start_row {
                            (start_pos.x, full_row_width)
                        } else if row == end_row {
                            (px(0.), end_pos.x)
                        } else {
                            (px(0.), full_row_width)
                        };
                        if x1 > x0 {
                            window.paint_quad(fill(
                                Bounds {
                                    origin: line_origin + point(x0, entry.top(row)),
                                    size: size(x1 - x0, entry.row_height(row)),
                                },
                                selection_fill,
                            ));
                        }
                    }
                }
            }
        }

        let stretch = justify::eligible(state, display_line, results.len());
        if (stretch && !wrapped.wrap_boundaries().is_empty()) || !entry.varied.is_empty() {
            let runs = line_runs(
                display_line,
                (font, &inline),
                (state.broken, callout),
                false,
            );
            let row_paint = justify::RowPaint {
                origin: line_origin,
                entry,
                stretch,
                font_size,
                runs: &runs,
            };
            justify::paint(
                wrapped,
                &display_line.display_text,
                &row_paint,
                (window, cx),
            );
        } else {
            let text_origin = line_origin + point(px(0.), entry.pad.0);
            let _ = wrapped.paint(text_origin, line_height, TextAlign::Left, None, window, cx);
        }
        if display_line
            .heading_level()
            .is_some_and(crate::settings::heading_rule)
        {
            // Under the text rows, above the space below the heading.
            let y = line_origin.y + entry.height - entry.pad.1 - px(1.);
            window.paint_quad(fill(
                Bounds {
                    origin: point(line_origin.x, y),
                    size: size(full_row_width, px(1.)),
                },
                rule,
            ));
        }
        let widgets = WidgetPaint {
            colors: &widget_colors,
            inline: &inline,
            font_size,
            entry,
            line_text,
            row_width: full_row_width,
            macros: state.macros,
            checkbox_radius,
        };
        paint_widgets(display_line, wrapped, line_origin, &widgets, (window, cx));

        if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
            let pos = justify::position_for_index(
                (state, wrapped, display_line),
                results.len(),
                display_line.display_offset_for_raw(cursor - line_range.start),
                line_height,
            )
            .unwrap_or(point(px(0.), px(0.)));
            let (pos, height) = entry.place(pos);
            window.paint_quad(fill(
                Bounds {
                    origin: line_origin + pos,
                    size: size(px(1.), height),
                },
                cursor_color,
            ));
        }
    }
}
