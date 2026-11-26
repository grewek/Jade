//! Code blocks inside floats and columns (#126).

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Pixels, TestAppContext, WindowOptions};

use super::super::Editor;
use super::region::shape_region;

/// Beside a float, a code block keeps the width it starts at, as a table or a
/// callout does, so one box fits it.
#[gpui_kit::test]
fn a_code_block_running_past_a_float_keeps_one_width(cx: &mut TestAppContext) {
    let code = "```\na\nb\nc\nd\ne\nf\n```";
    let source = format!("::: float right 40%\n\nbox\n\n:::\n\n{code}");
    let text = source.clone();
    let (window, editor) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
            cx.new(|cx| Editor::new(&text, cx))
        })
        .expect("open test window")
    });
    editor.update(cx, |e, _| {
        e.document_cache
            .borrow_mut()
            .view
            .set_column_width(Some(800.));
    });
    let widths: Vec<Pixels> = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let editor = editor.read(cx);
            let view = editor.last_view.clone().expect("a painted frame");
            let doc = editor.document_cache.borrow();
            let (region, split) = doc.side_by_side(0).expect("a float region");
            let shaped = shape_region(
                &source,
                &doc,
                (&region, &split),
                (0, false),
                (&view.font, view.font_size, view.line_height),
                (view.text_color.into(), None),
                window,
            );
            shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .filter(|l| !matches!(doc.line_roles()[l.line], super::LineRole::Plain))
                .map(|l| l.width)
                .collect()
        })
        .unwrap();
    assert_eq!(widths.len(), 8, "{widths:?}");
    assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");
}
