//! The caret rectangle an input method asks for, at offsets the caret is not
//! at. A line may have no drawn rows, and then there is no rectangle: the
//! subtraction on the way to one must never reach the line after it.

use super::super::Editor;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, EntityInputHandler, Pixels, TestAppContext, WindowOptions};

/// The rectangle at each of `offsets`, with the caret at the note's start.
/// The notes are ASCII, so an offset is the same in bytes and in UTF-16.
fn rectangles(
    cx: &mut TestAppContext,
    note: &'static str,
    offsets: impl Iterator<Item = usize>,
) -> Vec<Option<Bounds<Pixels>>> {
    assert!(note.is_ascii());
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, editor) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| Editor::new(note, cx))
        })
        .expect("open test window")
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        offsets
            .map(|at| {
                editor.update(cx, |editor, cx| {
                    editor.bounds_for_range(at..at, bounds, window, cx)
                })
            })
            .collect()
    })
    .unwrap()
}

/// Asserts that none of the lines `hidden` has a rectangle, at either end,
/// while the line reading `below` still has one.
fn assert_no_rectangle(cx: &mut TestAppContext, note: &'static str, hidden: &[usize]) {
    let lines: Vec<&str> = note.split('\n').collect();
    let start = |line: usize| lines[..line].iter().map(|l| l.len() + 1).sum::<usize>();
    let below = lines.iter().position(|l| *l == "below").expect("a line");
    let ends = hidden
        .iter()
        .flat_map(|&l| [start(l), start(l) + lines[l].len()]);
    let offsets: Vec<usize> = ends.chain([start(below)]).collect();
    let mut found = rectangles(cx, note, offsets.into_iter());
    assert!(found.pop().expect("below").is_some(), "{note:?}");
    for (k, rectangle) in found.into_iter().enumerate() {
        assert_eq!(rectangle, None, "line {} of {note:?}", hidden[k / 2]);
    }
}

#[gpui_kit::test]
fn a_line_inside_a_drawn_formula_block_has_no_caret_rectangle(cx: &mut TestAppContext) {
    assert_no_rectangle(cx, "top\n\n$$\nx + 1\ny + 2\n$$\n\nbelow", &[3, 4, 5]);
}

#[gpui_kit::test]
fn the_body_of_a_folded_callout_has_no_caret_rectangle(cx: &mut TestAppContext) {
    let note = "top\n\n> [!note]- Title\n> body\n> more\n\nbelow\n\n> [!tip]- At the end\n> last";
    assert_no_rectangle(cx, note, &[3, 4, 9]);
}

#[gpui_kit::test]
fn a_table_delimiter_row_has_no_caret_rectangle(cx: &mut TestAppContext) {
    assert_no_rectangle(cx, "top\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nbelow", &[3]);
}

/// The divider and the closing marker belong to no column.
#[gpui_kit::test]
fn a_region_line_in_no_column_has_no_caret_rectangle(cx: &mut TestAppContext) {
    let note = "top\n\n::: columns\n\nleft\n\n---\n\nright\n\n:::\n\nbelow";
    assert_no_rectangle(cx, note, &[6, 10]);
}

/// Every offset of a note with all the constructs that hide or replace text:
/// nothing on the way to a rectangle may underflow.
#[gpui_kit::test]
fn no_offset_of_a_note_panics_the_caret_rectangle(cx: &mut TestAppContext) {
    let note = "top\n\n# Head **bold** [[link|shown]]\n\n- [ ] task\n  1. nested\n\n\
> quote `code`\n>> deeper $a^2$\n\n> [!note]- Title\n> body\n\n> [!tip] Open\n> - item\n\n\
```rust\nfn main() {}\n```\n\n| a | **b** |\n|:--|--:|\n|  $x$ | [[y]] |\n\n$$\nx + 1\n$$\n\n\
---\n\n![[missing.png]]\n\n::: columns 60/40\n\n| c | d |\n|---|---|\n| 3 | 4 |\n\n---\n\n\
$$\ny\n$$\n\n:::\n\n::: aside right 30%\n\nfloat\n\n---\n\nflowing words\n\n:::\n\nbelow";
    let found = rectangles(cx, note, 0..=note.len() + 1);
    assert!(found.iter().any(Option::is_some));
    assert!(found.iter().any(Option::is_none));
}
