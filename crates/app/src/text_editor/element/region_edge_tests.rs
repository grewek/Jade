//! The lines at a region's edges (#116, #115, #118): a hand-typed region's
//! sides must be reachable, and neither typing nor deleting next to a marker
//! may end the region.

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

/// The blank line next to a region, outside it: typing there keeps the blank
/// line the marker needs, and the keys that would remove it do nothing.
#[gpui_kit::test]
fn editing_beside_a_region_keeps_it_a_region(cx: &mut TestAppContext) {
    let region = "::: columns\n\na\n\n---\n\nb\n\n:::";
    let source = format!("top\n\n{region}\n\nend");
    let before = "top\n".len();
    let after = source.len() - "\nend".len();
    // (caret, key or typed text, result)
    let cases = [
        (before, "x", format!("top\nx\n\n{region}\n\nend")),
        (after, "x", format!("top\n\n{region}\n\nx\nend")),
        (after, "enter", format!("top\n\n{region}\n\n\nend")),
        (after + 1, "backspace", source.clone()),
        ("top".len(), "delete", source.clone()),
    ];
    for (caret, what, expected) in cases {
        let (window, editor) = open(cx, &source);
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus_handle, cx);
                e.buffer.set_cursor(caret, false);
            });
            window.render_frame(cx);
        })
        .unwrap();
        if what.len() == 1 {
            type_text(cx, window, what);
        } else {
            press(cx, window, what);
        }
        let result = text_of(cx, &editor);
        assert_eq!(result, expected, "{what} at {caret}");
        let regions = editor.read_with(cx, |e, _| e.document_cache.borrow().columns().len());
        assert_eq!(regions, 1, "{what} at {caret}");
    }
}

/// A menu action or a divider drag keeps the caret with its text (typing used
/// to go to the region's first side, or onto a hidden marker line).
#[gpui_kit::test]
fn structure_edits_keep_the_caret_with_its_text(cx: &mut TestAppContext) {
    use gpui_kit::{Context, Window};
    type Edit = fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool;
    let columns = "intro\n\n::: columns\n\nleft\n\n---\n\nright\n\n:::\n\nend";
    let aside = "::: aside right 40%\n\nfloat\n\n---\n\nwords\n\n:::";
    let float = "one\n\n::: float right 30%\n\nbox\n\n:::\n\ntwo";
    let narrow_left: Edit = |e, w, c| e.set_region_ratio(0, vec![30, 70], w, c);
    let narrow_right: Edit = |e, w, c| e.set_region_ratio(0, vec![70, 30], w, c);
    let add: Edit = |e, w, c| e.add_region_column(0, w, c);
    let unwrap: Edit = |e, w, c| e.unwrap_region(0, w, c);
    let up: Edit = |e, w, c| e.move_float(false, w, c);
    let down: Edit = |e, w, c| e.move_float(true, w, c);
    // (source, the word the caret is in, the edit)
    let cases: [(&str, &str, Edit); 14] = [
        (columns, "right", narrow_left),
        (columns, "left", narrow_right),
        (columns, "intro", narrow_left),
        (columns, "end", narrow_left),
        (columns, "right", add),
        (columns, "right", unwrap),
        (columns, "end", unwrap),
        (aside, "words", Editor::flip_aside),
        (aside, "words", add),
        (float, "box", up),
        (float, "box", down),
        (float, "two", down),
        (float, "box", Editor::unwrap_float),
        (float, "two", Editor::unwrap_float),
    ];
    for (n, (source, word, edit)) in cases.into_iter().enumerate() {
        let (window, editor) = open(cx, source);
        // One byte into the word, so an offset error would show.
        let caret = source.find(word).unwrap() + 1;
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus_handle, cx);
                e.buffer.set_cursor(caret, false);
                assert!(edit(e, window, cx), "case {n}");
            });
            window.render_frame(cx);
        })
        .unwrap();
        type_text(cx, window, "|");
        let result = text_of(cx, &editor);
        let marked = format!("{}|{}", &word[..1], &word[1..]);
        assert!(result.contains(&marked), "case {n}: {result:?}");
    }
}

#[gpui_kit::test]
fn floating_a_block_puts_the_caret_at_its_start(cx: &mut TestAppContext) {
    for right in [true, false] {
        let (window, editor) = open(cx, "hello\n\nworld");
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus_handle, cx);
                assert!(e.make_float(right, window, cx));
            });
            window.render_frame(cx);
        })
        .unwrap();
        type_text(cx, window, "|");
        let result = text_of(cx, &editor);
        assert!(result.contains("\n\n|hello\n"), "right {right}: {result:?}");
    }
}

/// Undoing a structure edit puts the caret back where it was, not on the
/// marker line the edit rewrote, where the next key would end the region.
#[gpui_kit::test]
fn undoing_a_structure_edit_restores_the_caret(cx: &mut TestAppContext) {
    use gpui_kit::{Context, Window};
    type Edit = fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool;
    let columns = "::: columns\n\nleft\n\n---\n\nright\n\n:::";
    let float = "one\n\n::: float right 30%\n\nbox\n\n:::\n\ntwo";
    let cases: [(&str, &str, Edit); 5] = [
        (columns, "right", |e, w, c| {
            e.set_region_ratio(0, vec![30, 70], w, c)
        }),
        (columns, "right", |e, w, c| e.add_region_column(0, w, c)),
        (columns, "right", |e, w, c| e.unwrap_region(0, w, c)),
        (float, "box", |e, w, c| e.move_float(true, w, c)),
        (float, "box", Editor::unwrap_float),
    ];
    for (n, (source, word, edit)) in cases.into_iter().enumerate() {
        let (window, editor) = open(cx, source);
        let caret = source.find(word).unwrap() + 1;
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus_handle, cx);
                e.buffer.set_cursor(caret, false);
                assert!(edit(e, window, cx), "case {n}");
            });
            window.render_frame(cx);
        })
        .unwrap();
        press(cx, window, "ctrl-z");
        assert_eq!(text_of(cx, &editor), source, "case {n}");
        let after = editor.read_with(cx, |e, _| e.buffer().cursor());
        assert_eq!(after, caret, "case {n}");
    }
}
