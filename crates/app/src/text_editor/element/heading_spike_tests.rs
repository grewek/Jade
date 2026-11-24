//! #58 C1 spike: can a heading be shaped at a larger font size and still fit
//! the row model (whole multiples of the body row, monospace estimates)?
//! These pin what the heading steps rely on; the findings are in
//! ARCHITECTURE.md.

use super::super::Editor;
use crate::text_editor::layout::mono_char_width;
use gpui_kit::{AppContext, Font, Hsla, Pixels, TestAppContext, TextRun, WindowOptions, black, px};

const BODY: f32 = 16.0;
const ROW: f32 = 26.0;
const SCALES: [f32; 9] = [1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0];

fn mono(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Font) {
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

fn run(font: &Font, len: usize, bold: bool) -> TextRun {
    let mut font = font.clone();
    if bold {
        font.weight = gpui_kit::FontWeight::BOLD;
    }
    TextRun {
        len,
        font,
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// The advance of one character at `size`, and of a 40-char mixed line, as
/// multiples of the 16px ones: monospace advance must scale linearly or the
/// character-count estimate cannot be scaled by the heading factor.
#[gpui_kit::test]
fn advance_scales_linearly_with_font_size(cx: &mut TestAppContext) {
    let (window, font) = mono(cx);
    let text = "Heading with `code`, **bold** and wide: 0123456789";
    cx.update_window(window, |_, window, _| {
        let base = mono_char_width(&font, px(BODY), Hsla::default(), window);
        let line = |size: f32, bold: bool, w: &mut gpui_kit::Window| {
            w.text_system()
                .shape_line(text.into(), px(size), &[run(&font, text.len(), bold)], None)
                .width
        };
        let base_line = line(BODY, false, window);
        let mut worst = 0.0f32;
        for s in SCALES {
            let size = BODY * s;
            let one = mono_char_width(&font, px(size), Hsla::default(), window);
            let whole = line(size, false, window);
            let bold = line(size, true, window);
            let (a, b, c) = (
                f32::from(one) / (f32::from(base) * s),
                f32::from(whole) / (f32::from(base_line) * s),
                f32::from(bold) / f32::from(whole),
            );
            eprintln!("scale {s}: char {a:.4}  line {b:.4}  bold/regular {c:.4}");
            worst = worst
                .max((a - 1.0).abs())
                .max((b - 1.0).abs())
                .max((c - 1.0).abs());
        }
        // Per character: well under a thousandth of an em.
        assert!(worst < 0.002, "advance is not linear in size: {worst}");
    })
    .unwrap();
}

/// A heading shaped at `size` and wrapped at `width` takes the rows the
/// scaled estimate says, and its height is `rows * units * ROW` when the row
/// is `units` body rows (the existing `uniform` entry shape).
#[gpui_kit::test]
fn a_scaled_heading_wraps_as_the_scaled_estimate_says(cx: &mut TestAppContext) {
    let (window, font) = mono(cx);
    cx.update_window(window, |_, window, _| {
        let base = mono_char_width(&font, px(BODY), Hsla::default(), window);
        let mut bad = Vec::new();
        for s in SCALES {
            let size = px(BODY * s);
            let units = (s * 1.0).ceil() as usize; // body rows per heading row
            for width in [180.0f32, 333.0, 420.0, 750.0] {
                for words in [1usize, 4, 9, 16, 30] {
                    let text = (0..words)
                        .map(|w| "x".repeat(w * 7 % 11 + 1))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let wrapped = window
                        .text_system()
                        .shape_text(
                            text.clone().into(),
                            size,
                            &[run(&font, text.len(), true)],
                            Some(px(width)),
                            None,
                        )
                        .unwrap()
                        .pop()
                        .unwrap();
                    let rows = wrapped.wrap_boundaries().len() + 1;
                    let est = crate::text_editor::layout::wrapped_row_count(
                        &text,
                        Some(px(width)),
                        base * s,
                    );
                    let row = px(ROW) * units as f32;
                    let height: Pixels = wrapped.size(row).height;
                    assert_eq!(f32::from(height), f32::from(row) * rows as f32);
                    if rows != est {
                        bad.push((s, width as i32, words, est, rows));
                    }
                }
            }
        }
        // Word wrap needs a row more than the character count says, as for
        // body text (the cold estimate is corrected by `verify`), never fewer.
        eprintln!("estimate vs shaped (scale, width, words, estimate, shaped): {bad:?}");
        assert!(bad.iter().all(|&(.., est, rows)| rows >= est), "{bad:?}");
    })
    .unwrap();
}

/// How tall the text of one row is at each scale next to the row it gets
/// (`ceil(scale)` body rows): the slack a whole-row model costs.
#[gpui_kit::test]
fn whole_row_slack_per_scale(cx: &mut TestAppContext) {
    let (window, font) = mono(cx);
    cx.update_window(window, |_, window, _| {
        for s in SCALES {
            let size = px(BODY * s);
            let wrapped = window
                .text_system()
                .shape_text("Heading".into(), size, &[run(&font, 7, true)], None, None)
                .unwrap()
                .pop()
                .unwrap();
            let units = s.ceil();
            let text = f32::from(wrapped.ascent()) + f32::from(wrapped.descent());
            eprintln!(
                "scale {s}: text box {text:.1}px, row {} px ({} rows): {:.2} line-height",
                ROW * units,
                units,
                ROW * units / (BODY * s)
            );
        }
    })
    .unwrap();
}
