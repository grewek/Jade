//! #42 spike: can the app render itself headlessly with real fonts and the
//! wgpu renderer, is the output the same every time, and do cheap facts
//! (ink grid, row profile) tell a tall formula from a flat one? Ignored by
//! default (needs a GPU); run with
//! `cargo test -p jade-app capture_spike -- --ignored --nocapture`.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, HeadlessAppContext, Pixels, Size, WindowOptions, px, size};
use image::RgbaImage;

use crate::JadeApp;
use crate::assets::JadeAssets;
use crate::theme::ThemeEngine;

const NOTE: &str = "Big";

/// Renders `text` in the headless app twice and returns both frames.
fn render_twice(text: &str, window_size: Size<Pixels>) -> (RgbaImage, RgbaImage) {
    let dir = std::env::temp_dir().join(format!("jade-spike-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join(format!("{NOTE}.md")), text).expect("note");

    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(JadeAssets), || {
        gpui_kit::platform::current_headless_renderer()
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::app::install_fonts(cx);
        ThemeEngine::install(None, cx);
        cx.bind_keys(crate::text_editor::key_bindings());
    });
    let vault = dir.clone();
    let options = WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
            origin: gpui_kit::point(px(0.), px(0.)),
            size: window_size,
        })),
        ..Default::default()
    };
    let (window, _app) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| JadeApp::new(vault.clone(), window, cx))
            })
        })
        .expect("window");
    let frame = |cx: &mut HeadlessAppContext| {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("frame");
    };
    frame(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(NOTE, cx);
        window.render_frame(cx);
    })
    .expect("open note");
    frame(&mut cx);
    let first = cx.capture_screenshot(window).expect("first capture");
    frame(&mut cx);
    let second = cx.capture_screenshot(window).expect("second capture");
    let _ = std::fs::remove_dir_all(&dir);
    (first, second)
}

/// Ink (pixels that differ from the most common colour) per pixel row.
fn row_profile(img: &RgbaImage) -> Vec<u32> {
    let mut counts = std::collections::HashMap::new();
    for p in img.pixels() {
        *counts.entry(p.0).or_insert(0u32) += 1;
    }
    let background = counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c);
    (0..img.height())
        .map(|y| {
            (0..img.width())
                .filter(|&x| Some(img.get_pixel(x, y).0) != background)
                .count() as u32
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU"]
fn headless_capture_is_stable_and_shows_the_formula() {
    let flat = "| a | b |\n| - | - |\n| c | $x^2$ |\n";
    let tall = "| a | b |\n| - | - |\n| c | $\\frac{\\frac{a}{b}}{\\frac{c}{d}}$ |\n";
    let out = std::env::var("SPIKE_OUT").unwrap_or_else(|_| "/tmp".into());
    for (name, text) in [("flat", flat), ("tall", tall)] {
        let (a, b) = render_twice(text, size(px(900.), px(500.)));
        println!(
            "{name}: {}x{}, identical twice: {}",
            a.width(),
            a.height(),
            a == b
        );
        a.save(format!("{out}/spike-{name}.png")).expect("save");
        let profile = row_profile(&a);
        let inked: Vec<_> = profile
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(y, _)| y)
            .collect();
        println!(
            "{name}: ink rows {}..{} ({} rows with ink)",
            inked.first().copied().unwrap_or(0),
            inked.last().copied().unwrap_or(0),
            inked.len()
        );
        assert_eq!(a, b, "{name}: two renders of the same frame differ");
    }
}
