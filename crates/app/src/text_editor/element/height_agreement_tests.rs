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
    // Headings of every level and length, for the scaled counts (#58).
    for k in 1..60_usize {
        let level = "#".repeat(k % 6 + 1);
        let words: Vec<String> = (0..k).map(|w| "y".repeat(w * 5 % 9 + 1)).collect();
        text.push_str(&format!("{level} {}\n", words.join(" ")));
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

/// The cold estimate only positions what is off screen, so it need not be
/// exact, but it must stay close: a note's scrollbar and total height come
/// from it. It counts characters, so it misses the slack greedy word wrap
/// leaves at the end of a row: by up to a row in eight on a long paragraph.
/// Few lines may be off at all, and none by more than that.
#[gpui_kit::test]
fn the_cold_estimate_stays_close_to_the_shaped_count(cx: &mut TestAppContext) {
    let off = disagreements(cx, |_, _, _, _, _| {}, 0..usize::MAX);
    let lines = fixture().lines().count();
    let too_far: Vec<_> = off
        .iter()
        .filter(|&&(_, cached, shaped)| {
            (cached - shaped as i64).unsigned_abs() as usize > 1 + shaped / 8
        })
        .collect();
    assert!(too_far.is_empty(), "off by more than 1 + 1/8: {too_far:?}");
    assert!(
        off.len() * 12 <= lines,
        "{} of {lines} lines are off",
        off.len()
    );
}

/// A formula inside emphasis or strikethrough is shown as written, never drawn,
/// so it does not make its line tall; the cold estimate agrees with painting
/// (it once counted it, doubling the line).
#[gpui_kit::test]
fn a_formula_inside_emphasis_does_not_make_the_estimate_tall(cx: &mut TestAppContext) {
    let tail = " and more words here to wrap around the line a few times over and over again yes";
    let lines: Vec<String> = [
        "then $y^2$",
        "then **bold $y^2$ in**",
        "then *italic $y^2$ in*",
        "then ~~struck $y^2$ in~~",
        "then [[a note $y^2$]] and $y^2$",
    ]
    .iter()
    .map(|l| format!("{l}{tail}"))
    .collect();
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
    for line in lines {
        let text = format!("{line}\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let doc = DocumentCache::new(&text);
            let mut cache = RowCache::empty();
            let look = (&font, size, style.color);
            rows::rebuild_for_test(
                &mut cache,
                &text,
                &doc,
                (0, false),
                px(NARROW),
                look,
                window,
            );
            let shaped =
                rows::shaped_count_for_test(&text, &doc, 0, (0, false), px(NARROW), look, window);
            assert_eq!(rows::count_at(&cache, 0), shaped as i64, "{line}");
        })
        .unwrap();
    }
}

/// A `$$` block shown as a formula is counted without building its lines (the
/// first line holds the rows, the rest none); that equals what shaping gives,
/// whether the caret is outside the block or inside it.
#[gpui_kit::test]
fn the_cold_count_of_a_math_block_equals_shaping(cx: &mut TestAppContext) {
    let dir = env!("CARGO_MANIFEST_DIR");
    let text = std::fs::read_to_string(format!("{dir}/benches/fixtures/math_showcase.md")).unwrap();
    let inside = text.find("$$").unwrap() + 3;
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
    for (cursor, focused) in [(0, false), (inside, true)] {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let look = (&font, size, style.color);
            let doc = DocumentCache::new(&text);
            let mut cache = RowCache::empty();
            let state = (cursor, focused);
            rows::rebuild_for_test(&mut cache, &text, &doc, state, px(NARROW), look, window);
            let mut blocks = 0;
            for i in (0..doc.line_count()).filter(|&i| doc.math_block_at(i).is_some()) {
                blocks += 1;
                let shaped =
                    rows::shaped_count_for_test(&text, &doc, i, state, px(NARROW), look, window);
                assert_eq!(
                    rows::count_at(&cache, i),
                    shaped as i64,
                    "line {i}, caret {cursor}"
                );
            }
            assert!(blocks > 10, "the note has math blocks");
        })
        .unwrap();
    }
}

/// With heading sizes on, the cold estimate of a plain heading counts its wider
/// characters and its rows of several body rows: a short one exactly, a wrapped
/// one within the slack of word wrap (#58 C3).
#[gpui_kit::test]
fn the_cold_estimate_of_a_heading_follows_its_size(cx: &mut TestAppContext) {
    let mut t = crate::settings::Typography::default();
    t.heading.scale = [2.0, 1.75, 1.5, 1.25, 1.1, 1.0];
    crate::settings::set_body(&t);
    let off = disagreements(cx, |_, _, _, _, _| {}, 0..usize::MAX);
    crate::settings::set_body(&crate::settings::Typography::default());
    let text = fixture();
    let lines: Vec<&str> = text.lines().collect();
    let headings: Vec<_> = off
        .iter()
        .filter(|&&(i, ..)| lines[i].starts_with('#') && lines[i].contains(' '))
        .filter(|&&(i, cached, shaped)| {
            let units = shaped as i64 / 2;
            lines[i].len() < 20 || (cached - shaped as i64).abs() > units.max(2) * 2
        })
        .collect();
    assert!(headings.is_empty(), "headings off: {headings:?}");
}

/// Space above and below a heading adds whole rows to the estimate and to the
/// shaped count alike (#58 C5).
#[gpui_kit::test]
fn the_cold_estimate_of_a_spaced_heading_matches_shaping(cx: &mut TestAppContext) {
    let mut t = crate::settings::Typography::default();
    t.heading.scale = [2.0, 1.75, 1.5, 1.25, 1.1, 1.0];
    t.heading.space_above = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
    t.heading.space_below = [2.0, 0.0, 1.0, 1.0, 0.0, 0.0];
    crate::settings::set_body(&t);
    let off = disagreements(cx, |_, _, _, _, _| {}, 0..usize::MAX);
    let verified = disagreements(
        cx,
        |text, doc, cache, range, window| {
            let (font, size, color) = style_of(window);
            rows::verify_for_test(cache, text, doc, range, (&font, size, color), window);
        },
        0..usize::MAX,
    );
    crate::settings::set_body(&crate::settings::Typography::default());
    let text = fixture();
    let lines: Vec<&str> = text.lines().collect();
    let short: Vec<_> = off
        .iter()
        .filter(|&&(i, ..)| lines[i].starts_with('#') && lines[i].len() < 20)
        .collect();
    assert!(short.is_empty(), "short spaced headings off: {short:?}");
    assert!(
        verified.is_empty(),
        "verify disagrees: {:?}",
        &verified[..verified.len().min(5)]
    );
}
