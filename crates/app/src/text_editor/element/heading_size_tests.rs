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
