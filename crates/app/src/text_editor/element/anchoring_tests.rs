//! Making the lines in view exact can change the rows of lines above the
//! first visible one; the content under the reader must not move (#24).

use super::super::Editor;
use super::LineLayout;
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::rows::{self, Inputs, RowCache};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, TestAppContext, WindowOptions, point, px, size};
use std::cell::RefCell;
use std::rc::Rc;

#[gpui_kit::test]
fn rows_corrected_above_the_first_visible_line_are_reported_as_a_shift(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, _editor) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new("", cx))
        })
        .unwrap()
    });
    let text: String = (0..400).map(|i| format!("line {i}\n")).collect();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let width = px(800.);
        let doc_cache = Rc::new(RefCell::new(DocumentCache::new(&text)));
        let mut cache = RowCache::empty();
        rows::rebuild_for_test(
            &mut cache,
            &text,
            &doc_cache.borrow(),
            (0, false),
            width,
            (&font, font_size, style.color),
            window,
        );
        let layout = LineLayout {
            doc_cache: doc_cache.clone(),
            row_cache: Rc::new(RefCell::new(cache)),
            line_height,
        };
        // Scrolled so that line 200 is the first one in view.
        let viewport = Bounds {
            origin: point(px(0.), px(50.)),
            size: size(width, px(400.)),
        };
        let bounds = Bounds {
            origin: point(px(0.), px(50.) - line_height * 200.),
            size: size(width, line_height * 400.),
        };
        let inp = Inputs {
            text: &text,
            cursor: 0,
            focused: false,
            width,
            font: &font,
            font_size,
            text_color: style.color,
        };
        // Nothing wrong: no shift.
        let shift = layout.verify_anchored((bounds, viewport), &inp, window);
        assert_eq!(shift, px(0.));
        let top_before = layout.screen_y(bounds.origin.y, 200);

        // Line 190 (in the overscan above the view) was counted three rows
        // too tall; verifying shapes it again and its rows shrink by two.
        {
            let mut cache = layout.row_cache.borrow_mut();
            rows::correct_for_test(&mut cache, 190, 3);
            rows::forget_verified_for_test(&mut cache);
        }
        assert_eq!(
            layout.screen_y(bounds.origin.y, 200),
            top_before + line_height * 2.,
            "the wrong count moved line 200"
        );
        let shift = layout.verify_anchored((bounds, viewport), &inp, window);
        assert_eq!(shift, line_height * -2.);
        // Moving the origin by the shift keeps line 200 where the reader saw it
        // (with the wrong count), instead of two rows higher.
        let anchored = bounds.origin.y - shift;
        assert_eq!(
            layout.screen_y(anchored, 200),
            top_before + line_height * 2.
        );
        assert_eq!(layout.screen_y(bounds.origin.y, 200), top_before);
    })
    .unwrap();
}
