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
    group.finish();
}

fn bench_layout(c: &mut Criterion) {
    let stress = stress_text();
    let typical = typical_prefix(&stress);
    bench_open_note(c, "typical", typical);
    bench_open_note(c, "stress", &stress);
    bench_steady_state(c, "typical", typical);
    bench_steady_state(c, "stress", &stress);
}

criterion_group!(benches, bench_layout);
criterion_main!(benches);
