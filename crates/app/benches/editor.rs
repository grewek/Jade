//! Window-free editor internals on the stress fixture (11.8MB, ~320K lines)
//! and on a "typical" ~150KB prefix of it. Per-keystroke paths time only the
//! cache patch (the text edit itself happens outside the timed region), each
//! iteration being one edit, alternately an insert and its inverse.

use std::time::{Duration, Instant};

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use jade_app::bench_api::{Cache, EditorBuffer, Index};

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

/// Start of the first line at or after the document's midpoint.
fn middle_line_start(text: &str) -> usize {
    let mid = text.len() / 2;
    text[mid..].find('\n').map_or(0, |i| mid + i + 1)
}

/// The production cold path: all four classification passes in one build.
fn bench_cache_build(c: &mut Criterion) {
    let stress = stress_text();
    let typical = typical_prefix(&stress);
    let mut group = c.benchmark_group("document_cache_new");
    group.sample_size(10);
    group.bench_function("typical", |b| b.iter(|| Cache::new(black_box(typical))));
    group.bench_function("stress", |b| {
        b.iter(|| Cache::new(black_box(stress.as_str())))
    });
    group.finish();
}

/// Times `Cache::record_edit` only: each iteration applies `insert` at `at`
/// or removes it again, so the document returns to its start state.
fn bench_edit(c: &mut Criterion, name: &str, source: &str, at: Option<usize>, insert: &str) {
    let mut text = source.to_owned();
    let mut cache = Cache::new(&text);
    let at = at.unwrap_or_else(|| middle_line_start(&text));
    let mut inserted = false;
    c.bench_function(name, |b| {
        b.iter_custom(|iters| {
            let mut timed = Duration::ZERO;
            for _ in 0..iters {
                let (old_end, new_end) = if inserted {
                    text.replace_range(at..at + insert.len(), "");
                    (at + insert.len(), at)
                } else {
                    text.insert_str(at, insert);
                    (at, at + insert.len())
                };
                inserted = !inserted;
                let start = Instant::now();
                black_box(cache.record_edit(&text, at, old_end, new_end));
                timed += start.elapsed();
            }
            timed
        });
    });
}

fn bench_record_edit(c: &mut Criterion) {
    let stress = stress_text();
    let typical = typical_prefix(&stress);
    bench_edit(c, "record_edit/char/typical", typical, None, "x");
    bench_edit(c, "record_edit/newline/typical", typical, None, "\n");
    bench_edit(c, "record_edit/char/stress", &stress, None, "x");
    bench_edit(c, "record_edit/newline/stress", &stress, None, "\n");
    // Worst known case: a character typed on a callout header's line removes
    // the callout, which shifts every later callout count (see ARCHITECTURE.md).
    let header = stress.find("\n> [!").map(|i| i + 1);
    bench_edit(
        c,
        "record_edit/char_on_callout_header/stress",
        &stress,
        header,
        "x",
    );
}

fn bench_fenwick(c: &mut Criterion) {
    let stress = stress_text();
    let strides: Vec<i64> = stress
        .split_inclusive('\n')
        .map(|l| l.len() as i64)
        .collect();
    let mut index = Index::new(&strides);
    let mid = strides.len() / 2;
    let target = index.prefix_sum(mid);
    let mut group = c.benchmark_group("fenwick");
    group.bench_function("build", |b| b.iter(|| Index::new(black_box(&strides))));
    group.bench_function("prefix_sum", |b| {
        b.iter(|| index.prefix_sum(black_box(mid)))
    });
    group.bench_function("find", |b| b.iter(|| index.find(black_box(target))));
    group.bench_function("add_at", |b| {
        b.iter(|| {
            index.add_at(black_box(mid), 1);
            index.add_at(black_box(mid), -1);
        })
    });
    group.finish();
}

/// One insert at the middle of the document plus the delete that undoes it;
/// dominated by `String::insert` moving the tail (see ARCHITECTURE.md).
fn bench_buffer(c: &mut Criterion) {
    let stress = stress_text();
    let at = middle_line_start(&stress);
    let mut buffer = EditorBuffer::new(stress);
    buffer.set_cursor(at, false);
    c.bench_function("editor_buffer/insert_then_delete/stress", |b| {
        b.iter(|| {
            buffer.insert(black_box("x"));
            buffer.delete_backward();
        })
    });
}

criterion_group!(
    benches,
    bench_cache_build,
    bench_record_edit,
    bench_fenwick,
    bench_buffer
);
criterion_main!(benches);
