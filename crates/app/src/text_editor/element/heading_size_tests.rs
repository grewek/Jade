//! Headings shaped at their own size (#58 C2): the heading's rows are whole
//! body rows, and the caret, clicks and selection geometry read them through
//! the entry, as for any tall row.

use super::super::Editor;
use crate::settings::{self, HEADING_LEVELS, Typography};
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::layout::{LineState, entry_heights, shape_display_line};
use gpui_kit::{AppContext, Font, Pixels, TestAppContext, WindowOptions, point, px};

/// Sets the heading scales for a test and puts the defaults back after it.
struct Scales;

impl Scales {
    fn with(edit: impl FnOnce(&mut Typography)) -> Scales {
        let mut t = Typography::default();
        edit(&mut t);
        settings::set_body(&t);
        Scales
    }

    fn set(scale: [f32; HEADING_LEVELS]) -> Scales {
        let mut t = Typography::default();
        t.heading.scale = scale;
        settings::set_body(&t);
        Scales
    }
}

impl Drop for Scales {
    fn drop(&mut self) {
        settings::set_body(&Typography::default());
    }
}

const SCALES: [f32; HEADING_LEVELS] = [2.0, 1.75, 1.5, 1.25, 1.1, 1.0];

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
        .update_window(window.into(), |_, window, _| {
            let mut f = window.text_style().font();
            f.family = family;
            f
        })
        .unwrap();
    (window.into(), font)
}

/// The shaped heading `# ...` on line 1 (the caret is on line 0) at `width`:
/// its entry heights, the wrapped line and the body font size.
fn shape(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    font: &Font,
    text: &str,
    width: f32,
) -> (super::super::layout::EntryHeight, usize, Pixels) {
    let doc = DocumentCache::new(text);
    cx.update_window(window, |_, window, _| {
        let font_size = px(16.);
        let line_height = px(crate::settings::body_row_height());
        let state = LineState::of(&doc, 1, 0, true);
        let line = &text[doc.line_start_byte(1)..doc.line_end_byte(1)];
        let shaped = shape_display_line(
            line,
            &state,
            font,
            font_size,
            gpui_kit::black(),
            px(width),
            window,
        );
        assert_eq!(shaped.len(), 1);
        let entry = entry_heights(&shaped, line, (font_size, line_height), doc.macros())
            .next()
            .unwrap();
        let rows = shaped[0].0.wrap_boundaries().len() + 1;
        (entry, rows, shaped[0].0.width())
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_heading_row_is_its_levels_whole_body_rows(cx: &mut TestAppContext) {
    let _s = Scales::set(SCALES);
    let (window, font) = open(cx);
    // Units per row: round(scale), at least one.
    let expected = [2.0, 2.0, 2.0, 1.0, 1.0, 1.0];
    for (level, units) in (1..=HEADING_LEVELS).zip(expected) {
        let text = format!("caret\n{} Title", "#".repeat(level));
        let (entry, rows, _) = shape(cx, window, &font, &text, 900.);
        assert_eq!(rows, 1);
        assert_eq!(f32::from(entry.row), 26.0 * units, "level {level}");
        assert_eq!(f32::from(entry.height), 26.0 * units, "level {level}");
        assert!(entry.varied.is_empty());
    }
}

#[gpui_kit::test]
fn a_heading_is_wider_and_wraps_by_its_own_width(cx: &mut TestAppContext) {
    let (window, font) = open(cx);
    let text = "caret\n# one two three four five six seven eight nine ten";
    let (flat_entry, flat_rows, flat_width) = shape(cx, window, &font, text, 300.);
    assert_eq!(f32::from(flat_entry.row), 26.0);
    let _s = Scales::set(SCALES);
    let (entry, rows, width) = shape(cx, window, &font, text, 300.);
    assert!(rows > flat_rows, "{rows} vs {flat_rows}");
    assert!(width <= px(300.) && flat_width <= px(300.));
    // Every wrapped row is two body rows, so the height is rows * 52.
    assert_eq!(f32::from(entry.height), 52.0 * rows as f32);
    assert!(entry.varied.is_empty());
}

#[gpui_kit::test]
fn clicks_and_carets_in_a_wrapped_heading_round_trip(cx: &mut TestAppContext) {
    let _s = Scales::set(SCALES);
    let (window, font) = open(cx);
    let text = "caret\n# one two three four five six seven eight nine ten";
    let doc = DocumentCache::new(text);
    cx.update_window(window, |_, window, _| {
        let (font_size, row) = (px(16.), px(26.));
        let state = LineState::of(&doc, 1, 0, true);
        let line = &text[doc.line_start_byte(1)..doc.line_end_byte(1)];
        let shaped = shape_display_line(
            line,
            &state,
            &font,
            font_size,
            gpui_kit::black(),
            px(300.),
            window,
        );
        let entry = entry_heights(&shaped, line, (font_size, row), doc.macros())
            .next()
            .unwrap();
        let (wrapped, display) = &shaped[0];
        assert!(wrapped.wrap_boundaries().len() >= 2);
        let mut last_y = px(-1.);
        for at in
            (0..display.display_text.len()).filter(|i| display.display_text.is_char_boundary(*i))
        {
            // Where the caret of display byte `at` is painted ...
            let layout = wrapped.position_for_index(at, entry.row).unwrap();
            let (placed, caret_height) = entry.place(layout);
            assert_eq!(caret_height, px(52.));
            assert!(placed.y >= last_y, "rows go down");
            last_y = placed.y;
            // ... and a click a little inside that row, to the right of it,
            // lands on the same byte (monospace: half an advance in).
            let half = wrapped.width() / display.display_text.len() as f32 / 2.0;
            let click = point(layout.x + half, entry.layout_y(placed.y + px(10.)));
            let hit = wrapped
                .closest_index_for_position(click, entry.row)
                .unwrap_or_else(|i| i);
            let next = display.display_text.len().min(at + 1);
            assert!(hit == at || hit == next, "click at {at} hit {hit}");
        }
        // The last row begins two heading rows (4 body rows) per row down.
        assert_eq!(entry.top(2), px(104.));
        assert_eq!(entry.row_at(px(110.)), 2);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_defaults_leave_headings_at_body_size(cx: &mut TestAppContext) {
    let (window, font) = open(cx);
    let text = "caret\n# Title\nplain";
    let (entry, rows, width) = shape(cx, window, &font, text, 900.);
    assert_eq!(
        (rows, f32::from(entry.row), f32::from(entry.height)),
        (1, 26.0, 26.0)
    );
    let _s = Scales::set(SCALES);
    let (_, _, scaled) = shape(cx, window, &font, text, 900.);
    assert!((f32::from(scaled) / f32::from(width) - 2.0).abs() < 1e-3);
}

#[gpui_kit::test]
fn space_around_a_heading_is_whole_rows_the_text_sits_inside(cx: &mut TestAppContext) {
    // 0.8 above rounds to one row, 1.6 below to two; level 1 is two rows tall.
    let _s = Scales::with(|t| {
        t.heading.scale = SCALES;
        t.heading.space_above[0] = 0.8;
        t.heading.space_below[0] = 1.6;
    });
    let (window, font) = open(cx);
    let (entry, rows, _) = shape(cx, window, &font, "caret\n# Title", 900.);
    assert_eq!(rows, 1);
    assert_eq!(entry.pad, (px(26.), px(52.)));
    assert_eq!(f32::from(entry.height), 26.0 + 52.0 + 52.0);
    // The text starts below the space above, and clicks in it land on row 0.
    assert_eq!(entry.top(0), px(26.));
    assert_eq!(entry.place(point(px(5.), px(0.))).0, point(px(5.), px(26.)));
    assert_eq!(entry.row_at(px(3.)), 0);
    assert_eq!(entry.layout_y(px(26. + 10.)), px(10.));
    assert_eq!(entry.layout_y(px(3.)), px(0.));
    // Another level has none.
    let (plain, ..) = shape(cx, window, &font, "caret\n## Title", 900.);
    assert_eq!(plain.pad, (px(0.), px(0.)));
}

#[test]
fn the_rule_and_space_defaults_change_nothing() {
    let t = Typography::default();
    settings::set_body(&t);
    for level in 1..=6 {
        assert_eq!(settings::heading_space_rows(level), (0, 0));
        assert!(!settings::heading_rule(level));
    }
}

/// The shaped display text and entry of line 1 of `text` at `width`.
fn shown(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    font: &Font,
    text: &str,
    width: f32,
) -> (String, super::super::layout::EntryHeight) {
    let doc = DocumentCache::new(text);
    cx.update_window(window, |_, window, _| {
        let (font_size, row) = (px(16.), px(26.));
        let state = LineState::of(&doc, 1, 0, true);
        let line = &text[doc.line_start_byte(1)..doc.line_end_byte(1)];
        let shaped = shape_display_line(
            line,
            &state,
            font,
            font_size,
            gpui_kit::black(),
            px(width),
            window,
        );
        let entry = entry_heights(&shaped, line, (font_size, row), doc.macros())
            .next()
            .unwrap();
        (shaped[0].1.display_text.clone(), entry)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_formula_in_a_heading_is_sized_with_it(cx: &mut TestAppContext) {
    let (window, font) = open(cx);
    let text = "caret\n# $\\frac{\\frac{a}{b}}{\\frac{c}{d}}$";
    let (_, body) = shown(cx, window, &font, text, 900.);
    let _s = Scales::set([3.0; HEADING_LEVELS]);
    let (_, big) = shown(cx, window, &font, text, 900.);
    // The same formula at three times the size needs more than three times a
    // body row's height only if it is drawn at the heading's size.
    assert!(
        big.height >= body.height * 3.0 - px(0.5),
        "{:?} vs {:?}",
        big.height,
        body.height
    );
    let (_, plain) = shown(cx, window, &font, "caret\n# word", 900.);
    assert_eq!(plain.height, px(78.));
}

#[gpui_kit::test]
fn a_heading_formula_wider_than_its_row_is_shown_as_source(cx: &mut TestAppContext) {
    let (window, font) = open(cx);
    let text = "caret\n# $x^2+y^2+z^2+w^2$";
    // 16px text is 9.6px per column: 40 columns fit 400px at body size but
    // not at three times the size (13 columns).
    let (flat, _) = shown(cx, window, &font, text, 400.);
    assert!(!flat.contains('$'), "drawn at body size: {flat:?}");
    let _s = Scales::set([3.0; HEADING_LEVELS]);
    let (big, _) = shown(cx, window, &font, text, 150.);
    assert!(big.contains('$'), "too wide at heading size: {big:?}");
}

/// Entering a heading changes neither its row height nor its spacing, and its
/// text stays at the heading's size.
#[gpui_kit::test]
fn the_caret_line_of_a_heading_keeps_its_height(cx: &mut TestAppContext) {
    let _s = Scales::with(|t| {
        t.heading.scale = SCALES;
        t.heading.space_above[0] = 1.0;
        t.heading.space_below[0] = 1.0;
    });
    let (window, font) = open(cx);
    let text = "caret\n# Title\nplain";
    let doc = DocumentCache::new(text);
    let measure = |cx: &mut TestAppContext, cursor: usize| {
        cx.update_window(window, |_, window, _| {
            let (font_size, row) = (px(16.), px(26.));
            let state = LineState::of(&doc, 1, cursor, true);
            let line = &text[doc.line_start_byte(1)..doc.line_end_byte(1)];
            let shaped = shape_display_line(
                line,
                &state,
                &font,
                font_size,
                gpui_kit::black(),
                px(900.),
                window,
            );
            let entry = entry_heights(&shaped, line, (font_size, row), doc.macros())
                .next()
                .unwrap();
            (
                entry,
                shaped[0].0.size(row).width / shaped[0].1.display_text.len() as f32,
            )
        })
        .unwrap()
    };
    let (shown, shown_advance) = measure(cx, 0);
    let (editing, editing_advance) = measure(cx, doc.line_start_byte(1) + 2);
    assert_eq!(editing.height, shown.height);
    assert_eq!(editing.pad, shown.pad);
    assert!(
        (f32::from(editing_advance) - f32::from(shown_advance)).abs() < 0.01,
        "same advance per character: {editing_advance:?} vs {shown_advance:?}"
    );
}
