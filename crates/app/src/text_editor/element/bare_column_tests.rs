//! A hand-typed region has a single blank line per side (#116): each side
//! must be reachable by click, and typing there must keep the region a region.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, ClipboardItem, Entity, EntityInputHandler, TestAppContext,
    WindowOptions, point, px,
};

use super::super::Editor;

const COLUMNS: &str = "::: columns\n\n---\n\n:::";
const ASIDE: &str = "::: aside right 40%\n\n---\n\n:::";

fn open(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<Editor>) {
    let text = text.to_owned();
    let (window, editor) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        cx.bind_keys(crate::text_editor::key_bindings());
        gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
            cx.new(|cx| Editor::new(&text, cx))
        })
        .expect("open test window")
    });
    // Without the app's scroll container no paint measures a width.
    editor.update(cx, |e, _| {
        e.document_cache
            .borrow_mut()
            .view
            .set_column_width(Some(800.));
    });
    (window, editor)
}

/// Clicks the first row at `x`, a share of the editor's width.
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, x: f32) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let width = window.find("editor").bounds().size.width;
        window.click_at("editor", point(width * x, px(4.)), cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn type_text(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| {
        window.input(text, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| {
        window.press(key, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn text_of(cx: &mut TestAppContext, editor: &Entity<Editor>) -> String {
    editor.read_with(cx, |e, _| e.buffer().text().to_owned())
}

/// The part of the one region the caret is in.
fn caret_column(cx: &mut TestAppContext, editor: &Entity<Editor>) -> Option<usize> {
    editor.read_with(cx, |e, _| {
        let doc = e.document_cache.borrow();
        assert_eq!(doc.columns().len(), 1, "in {:?}", e.buffer().text());
        let line = doc.line_at(e.buffer().cursor());
        doc.side_by_side(line)
            .and_then(|(region, _)| region.column_of(line))
    })
}

#[gpui_kit::test]
fn each_side_of_a_typed_region_takes_a_click_and_typing(cx: &mut TestAppContext) {
    // (source, [(click x, part it reaches, the source after typing "x" there)])
    let cases = [
        (
            COLUMNS,
            [
                (0.2, 0, "::: columns\n\nx\n\n---\n\n:::"),
                (0.8, 1, "::: columns\n\n---\n\nx\n\n:::"),
            ],
        ),
        // The float is on the right, the text (part 1) on the left.
        (
            ASIDE,
            [
                (0.2, 1, "::: aside right 40%\n\n---\n\nx\n\n:::"),
                (0.8, 0, "::: aside right 40%\n\nx\n\n---\n\n:::"),
            ],
        ),
    ];
    for (source, sides) in cases {
        let (window, editor) = open(cx, source);
        for (x, part, typed) in sides {
            click(cx, window, x);
            assert_eq!(caret_column(cx, &editor), Some(part), "click at {x}");
            type_text(cx, window, "x");
            assert_eq!(text_of(cx, &editor), typed);
            assert_eq!(caret_column(cx, &editor), Some(part));
            press(cx, window, "ctrl-z");
            assert_eq!(text_of(cx, &editor), source, "one undo step");
        }
    }
}

#[gpui_kit::test]
fn a_typed_float_block_takes_a_click_and_typing(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, "::: float right 40%\n\n:::\n\ntext");
    click(cx, window, 0.8);
    type_text(cx, window, "x");
    assert_eq!(
        text_of(cx, &editor),
        "::: float right 40%\n\nx\n\n:::\n\ntext"
    );
    assert_eq!(caret_column(cx, &editor), Some(0));
}

#[gpui_kit::test]
fn enter_tab_and_paste_keep_a_bare_column_in_the_region(cx: &mut TestAppContext) {
    let cases = [
        ("enter", "::: columns\n\n---\n\n\n\n\n:::"),
        ("tab", "::: columns\n\n---\n\n\t\n\n:::"),
        ("ctrl-v", "::: columns\n\n---\n\na\nb\n\n:::"),
    ];
    for (key, expected) in cases {
        let (window, editor) = open(cx, COLUMNS);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("a\nb".into())));
        click(cx, window, 0.8);
        press(cx, window, key);
        assert_eq!(text_of(cx, &editor), expected, "{key}");
        assert_eq!(caret_column(cx, &editor), Some(1), "{key}");
        type_text(cx, window, "z");
        assert_eq!(caret_column(cx, &editor), Some(1), "typing after {key}");
    }
}

#[gpui_kit::test]
fn composing_in_a_bare_column_marks_only_the_composed_text(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, COLUMNS);
    click(cx, window, 0.8);
    cx.update_window(window, |_, window, cx| {
        editor.update(cx, |e, cx| {
            e.replace_and_mark_text_in_range(None, "^", None, window, cx);
        });
        window.render_frame(cx);
        editor.update(cx, |e, cx| e.replace_text_in_range(None, "â", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(text_of(cx, &editor), "::: columns\n\n---\n\nâ\n\n:::");
    assert_eq!(caret_column(cx, &editor), Some(1));
}

#[gpui_kit::test]
fn left_and_right_reach_both_bare_columns(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, COLUMNS);
    click(cx, window, 0.2);
    press(cx, window, "right");
    assert_eq!(caret_column(cx, &editor), Some(1));
    press(cx, window, "left");
    assert_eq!(caret_column(cx, &editor), Some(0));
}

/// An empty region put on a blank line (#115): padded away from the text
/// around it, each part reachable, and one undo step.
#[gpui_kit::test]
fn an_empty_region_can_be_inserted_on_a_blank_line(cx: &mut TestAppContext) {
    use super::ColumnMenu;
    use gpui_kit::{Context, Window};
    type Insert = fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool;
    let columns: Insert = Editor::insert_columns;
    let float: Insert = |e, w, c| e.insert_float(true, w, c);
    // (source, caret, entry, result)
    let cases = [
        ("", 0, columns, "::: columns\n\n\n\n---\n\n\n\n:::"),
        (
            "a\n\nb",
            2,
            columns,
            "a\n\n::: columns\n\n\n\n---\n\n\n\n:::\n\nb",
        ),
        (
            "a\n",
            2,
            float,
            "a\n\n::: aside right 40%\n\n\n\n---\n\n\n\n:::",
        ),
    ];
    for (source, caret, insert, expected) in cases {
        let (window, editor) = open(cx, source);
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus_handle, cx);
                e.buffer.set_cursor(caret, false);
                assert!(e.column_menu_state() == Some(ColumnMenu::Blank));
                assert!(insert(e, window, cx));
            });
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(text_of(cx, &editor), expected);
        assert_eq!(caret_column(cx, &editor), Some(0));
        type_text(cx, window, "x");
        assert_eq!(text_of(cx, &editor).len(), expected.len() + 1);
        assert_eq!(caret_column(cx, &editor), Some(0));
        press(cx, window, "ctrl-z");
        press(cx, window, "ctrl-z");
        assert_eq!(text_of(cx, &editor), source);
    }
}

#[gpui_kit::test]
fn a_blank_line_of_fenced_code_offers_no_region(cx: &mut TestAppContext) {
    let (_, editor) = open(cx, "```\n\n```");
    editor.update(cx, |e, _| {
        e.buffer.set_cursor(4, false);
        assert!(e.column_menu_state().is_none());
    });
}
