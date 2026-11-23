//! The row cache's counts for the lines in view equal what shaping them gives,
//! whatever produced them: a cold estimate, a verify or an edit patch (#24).
//! Painting positions boxes and bands from the cache, so a count that differs
//! from the shaped one is a frame drawn out of line.

use super::super::Editor;
use crate::text_editor::doc::DocumentCache;
use crate::text_editor::rows::{self, RowCache};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Pixels, TestAppContext, WindowOptions, px};
use std::ops::Range;

/// Lines of the stress fixture the tests use: all its constructs, quickly.
const LINES: usize = 3000;

/// A narrow editor: long lines wrap, which is where the monospace estimate
/// and word wrapping disagree.
const NARROW: f32 = 420.;

fn fixture() -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    let mut text = String::new();
    for name in ["large_markdown_stress_test.md", "math_showcase.md"] {
        let full = std::fs::read_to_string(format!("{dir}/benches/fixtures/{name}")).unwrap();
        for line in full.lines().take(LINES) {
            text.push_str(line);
            text.push('\n');
        }
    }
    // Paragraphs of words of every length: where the greedy word wrap needs
    // a row more than the characters alone say.
    for k in 1..200_usize {
        let words: Vec<String> = (0..k).map(|w| "x".repeat(w * 7 % 11 + 1)).collect();
        text.push_str(&words.join(" "));
        text.push('\n');
    }
    text
}

/// Lines in `range` (not in a side-by-side region, whose rows are the
/// region's) whose cached count differs from the shaped one, as
/// `(line, cached, shaped)`.
fn disagreements(
    cx: &mut TestAppContext,
    build: impl FnOnce(&str, &DocumentCache, &mut RowCache, Range<usize>, &mut gpui_kit::Window)
    + 'static,
    range: Range<usize>,
) -> Vec<(usize, i64, usize)> {
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
    let text = fixture();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let width: Pixels = px(NARROW);
        let mut doc = DocumentCache::new(&text);
        doc.view.set_column_width(Some(NARROW));
        let mut cache = RowCache::empty();
        rows::rebuild_for_test(
            &mut cache,
            &text,
            &doc,
            (0, false),
            width,
            (&font, font_size, style.color),
            window,
        );
        let range = range.start..range.end.min(doc.line_count());
        build(&text, &doc, &mut cache, range.clone(), window);
        let mut off = Vec::new();
        for i in range.filter(|&i| doc.side_by_side(i).is_none()) {
            let shaped = rows::shaped_count_for_test(
                &text,
                &doc,
                i,
                (0, false),
                width,
                (&font, font_size, style.color),
                window,
            );
            let cached = rows::count_at(&cache, i);
            if cached != shaped as i64 {
                off.push((i, cached, shaped));
            }
        }
        off
    })
    .unwrap()
}

fn style_of(window: &gpui_kit::Window) -> (gpui_kit::Font, Pixels, gpui_kit::Hsla) {
    let style = window.text_style();
    (
        style.font(),
        style.font_size.to_pixels(window.rem_size()),
        style.color,
    )
}

#[gpui_kit::test]
fn verifying_a_window_makes_every_line_in_it_exact(cx: &mut TestAppContext) {
    let off = disagreements(
        cx,
        |text, doc, cache, range, window| {
            let (font, size, color) = style_of(window);
            rows::verify_for_test(cache, text, doc, range, (&font, size, color), window);
        },
        0..usize::MAX,
    );
    assert!(
        off.is_empty(),
        "{} lines differ, first: {:?}",
        off.len(),
        &off[..off.len().min(5)]
    );
}

#[gpui_kit::test]
fn patching_a_few_lines_makes_them_exact(cx: &mut TestAppContext) {
    let off = disagreements(
        cx,
        |text, doc, cache, range, window| {
            let (font, size, color) = style_of(window);
            rows::patch_for_test(
                cache,
                text,
                doc,
                range,
                (0, false),
                (&font, size, color),
                window,
            );
        },
        0..60,
    );
    assert!(
        off.is_empty(),
        "{} lines differ, first: {:?}",
        off.len(),
        &off[..off.len().min(5)]
    );
}
