//! Whole frames of the real app on GPUI's headless test platform: layout and
//! paint run, the GPU submit does not. `open_note` is the one-time cold open
//! (click, read, build the caches, first frame, including `RowHeightCache::
//! rebuild`); the rest are the steady-state per-frame costs the 16.66ms bar
//! applies to. Run with `--features bench-support`.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use jade_app::headless::HeadlessApp;

const TYPICAL_BYTES: usize = 150 * 1024;

fn stress_text() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benches/fixtures/large_markdown_stress_test.md"
    );
    match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => panic!("cannot read the stress fixture {path}: {err}"),
    }
}

/// The first ~`TYPICAL_BYTES` of `text`, cut after a newline.
fn typical_prefix(text: &str) -> &str {
    let tail = &text[TYPICAL_BYTES..];
    let end = TYPICAL_BYTES + tail.find('\n').map_or(tail.len(), |i| i + 1);
    &text[..end]
}

/// `text` with an image embed after every 200th line (outside tables), so a
/// block is always near the viewport and the row cache holds thousands of them.
fn with_image_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 200);
    for (i, line) in text.lines().enumerate() {
        out.push_str(line);
        out.push('\n');
        if i % 200 == 199 && !line.starts_with('|') {
            out.push_str("![[img/photo.png]]\n");
        }
    }
    out
}

fn bench_open_note(c: &mut Criterion, name: &str, text: &str) {
    let mut group = c.benchmark_group("open_note");
    group.sample_size(10);
    group.bench_function(name, |b| {
        b.iter_custom(|iters| {
            let mut timed = Duration::ZERO;
            for _ in 0..iters {
                let mut app = HeadlessApp::new(text);
                let start = Instant::now();
                app.open_note();
                timed += start.elapsed();
            }
            timed
        });
    });
    group.finish();
}

fn bench_steady_state(c: &mut Criterion, name: &str, text: &str) {
    let mut app = HeadlessApp::new(text);
    app.open_note();
    app.focus_editor();

    let mut group = c.benchmark_group(format!("frame/{name}"));
    group.bench_function("idle", |b| b.iter(|| app.frame()));

    // One character typed or erased, then the frame it causes; alternating
    // keeps the note the same size.
    let mut typed = false;
    group.bench_function("keystroke_and_frame", |b| {
        b.iter(|| {
            if typed {
                app.key_and_frame("backspace", None);
            } else {
                app.key_and_frame("x", Some("x"));
            }
            typed = !typed;
        });
    });

    let mut step = 0u32;
    group.bench_function("scroll_and_frame", |b| {
        b.iter(|| {
            step = (step + 1) % 100;
            app.scroll_and_frame(step as f32 / 100.0);
        });
    });

    // A sidebar drag step: the wrap width changes every frame, so the row
    // cache rebuilds in full each time.
    let mut wide = false;
    group.bench_function("sidebar_resize_and_frame", |b| {
        b.iter(|| {
            wide = !wide;
            app.resize_sidebar_and_frame(if wide { 300.0 } else { 260.0 });
        });
    });

    // A theme switch: widget colors and the palette change, layout does not,
    // so no row cache may rebuild.
    let mut dark = false;
    group.bench_function("theme_switch_and_frame", |b| {
        b.iter(|| {
            dark = !dark;
            app.set_theme_and_frame(if dark {
                "catppuccin-mocha"
            } else {
                "catppuccin-latte"
            });
        });
    });
    group.finish();
}

fn bench_layout(c: &mut Criterion) {
    let stress = stress_text();
    let typical = typical_prefix(&stress);
    bench_open_note(c, "typical", typical);
    bench_open_note(c, "stress", &stress);
    bench_steady_state(c, "typical", typical);
    bench_steady_state(c, "stress", &stress);
    bench_steady_state(c, "stress_blocks", &with_image_blocks(&stress));
}

criterion_group!(benches, bench_layout);
criterion_main!(benches);
