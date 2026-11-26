//! Seeded random typing and moving inside every kind of region, with real key
//! dispatch (the hardening pass, #118). Without a selection, ordinary editing
//! keys, code fences included (#126), must never break the region or leave the
//! caret on a hidden line.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};

use super::super::Editor;
use super::super::editor::is_hidden_line;
use super::super::test_support::Rng;

const SOURCES: [&str; 6] = [
    "<!-- jade:columns -->\n\n<!-- jade:next -->\n\n<!-- jade:end -->",
    "intro\n\n<!-- jade:columns -->\n\nleft text\n\n<!-- jade:next -->\n\n\nright\n\n\n<!-- jade:end -->\n\noutro",
    "<!-- jade:columns -->\n\na\n\n<!-- jade:next -->\n\n<!-- jade:next -->\n\nc\n\n<!-- jade:end -->",
    "<!-- jade:aside right 40% -->\n\n<!-- jade:next -->\n\n<!-- jade:end -->",
    "<!-- jade:aside left 30% -->\n\nfloat\n\n<!-- jade:next right 20% -->\n\n<!-- jade:next -->\n\nsome text\n\n<!-- jade:end -->\n\nafter",
    "<!-- jade:float right 40% -->\n\nF\n\n<!-- jade:end -->\n\nfirst paragraph\n\nsecond",
];

const KEYS: [&str; 11] = [
    "left",
    "right",
    "up",
    "down",
    "home",
    "end",
    "enter",
    "backspace",
    "delete",
    "ctrl-left",
    "ctrl-right",
];

#[gpui_kit::test]
fn editing_inside_regions_keeps_them_whole(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        cx.bind_keys(crate::text_editor::key_bindings());
    });
    for (n, source) in SOURCES.iter().enumerate() {
        for seed in 1..=4u64 {
            let (window, editor) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                    cx.new(|cx| Editor::new(*source, cx))
                })
                .expect("open test window")
            });
            editor.update(cx, |e, _| {
                e.document_cache
                    .borrow_mut()
                    .view
                    .set_column_width(Some(1000.));
            });
            let regions = editor.read_with(cx, |e, _| e.document_cache.borrow().columns().len());
            let mut rng = Rng::new(seed * 7919 + n as u64);
            let mut log = Vec::new();
            cx.update_window(window, |_, window, cx| {
                editor.update(cx, |e, cx| {
                    window.focus(&e.focus_handle, cx);
                    let doc = e.document_cache.borrow();
                    // Start on the first line a caret may be on, in a region.
                    let line = (0..doc.line_count())
                        .find(|&l| {
                            doc.side_by_side(l)
                                .is_some_and(|(r, _)| r.column_of(l).is_some())
                        })
                        .expect("a column line");
                    let at = doc.line_start_byte(line);
                    drop(doc);
                    e.buffer.set_cursor(at, false);
                });
                window.render_frame(cx);
            })
            .unwrap();
            for _ in 0..80 {
                let step = rng.below(KEYS.len() + 5);
                let what = KEYS
                    .get(step)
                    .copied()
                    .unwrap_or(["a", "```", " ", "é", "~~~"][step % 5]);
                log.push(what);
                cx.update_window(window, |_, window, cx| {
                    if step < KEYS.len() {
                        window.press(what, cx);
                    } else {
                        window.input(what, cx);
                    }
                    window.render_frame(cx);
                })
                .unwrap();
                editor.read_with(cx, |e, _| {
                    let doc = e.document_cache.borrow();
                    let at = format!("source {n} seed {seed} {log:?}\n{:?}", e.buffer().text());
                    assert_eq!(doc.columns().len(), regions, "region lost: {at}");
                    let line = doc.line_at(e.buffer().cursor());
                    assert!(
                        !is_hidden_line(&doc, line),
                        "caret on hidden line {line}: {at}"
                    );
                });
            }
            for _ in 0..80 {
                cx.update_window(window, |_, window, cx| window.press("ctrl-z", cx))
                    .unwrap();
            }
            editor.read_with(cx, |e, _| {
                assert_eq!(
                    e.buffer().text(),
                    *source,
                    "undo: source {n} seed {seed} {log:?}"
                );
            });
        }
    }
}
