//! Heap allocations per operation on the stress fixture, with an upper bound
//! on each (STYLE.md rule 8): a call that starts allocating in proportion to
//! the document fails this bench instead of showing up as a vague slowdown.
//! Plain `main` (no criterion): a counting allocator would distort timings.
//! Run with `cargo bench -p jade-app --features bench-support --bench
//! allocations`.

use jade_app::bench_api::{Cache, Index, Links};
use jade_app::headless::{AllocSnapshot, CountingAllocator, HeadlessApp, reset, snapshot};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

struct Report {
    failures: usize,
}

impl Report {
    /// Runs `op` once after a warm-up call (scratch buffers fill on first
    /// use) and checks the bytes it requested against `max_bytes`.
    fn check(&mut self, name: &str, max_bytes: usize, mut op: impl FnMut()) {
        op();
        reset();
        op();
        self.report(name, max_bytes, snapshot());
    }

    /// Like [`Self::check`] for an operation that measures itself (inside
    /// its own window update) and returns the snapshot.
    fn check_snapshot(
        &mut self,
        name: &str,
        max_bytes: usize,
        mut op: impl FnMut() -> AllocSnapshot,
    ) {
        op();
        let snap = op();
        self.report(name, max_bytes, snap);
    }

    fn report(&mut self, name: &str, max_bytes: usize, snap: AllocSnapshot) {
        let ok = snap.total <= max_bytes;
        println!(
            "{:<44} {:>9} bytes in {:>5} allocations (max {max_bytes}) {}",
            name,
            snap.total,
            snap.count,
            if ok { "ok" } else { "OVER" }
        );
        if !ok {
            self.failures += 1;
        }
    }
}

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

fn middle_line_start(text: &str) -> usize {
    let mid = text.len() / 2;
    text[mid..].find('\n').map_or(0, |i| mid + i + 1)
}

/// Applies `insert` at `at` (or removes it again), then reports the edit.
fn toggle_edit(cache: &mut Cache, text: &mut String, at: usize, insert: &str, inserted: &mut bool) {
    let (old_end, new_end) = if *inserted {
        text.replace_range(at..at + insert.len(), "");
        (at + insert.len(), at)
    } else {
        text.insert_str(at, insert);
        (at, at + insert.len())
    };
    *inserted = !*inserted;
    cache.record_edit(text, at, old_end, new_end);
}

fn cache_edits(report: &mut Report, stress: &str) {
    let header = stress.find("\n> [!").map_or(0, |i| i + 1);
    let cases = [
        ("record_edit char", None, "x", 64),
        ("record_edit newline", None, "\n", 64),
        (
            "record_edit char on a callout header",
            Some(header),
            "x",
            256,
        ),
    ];
    for (name, at, insert, max) in cases {
        let mut text = stress.to_owned();
        let mut cache = Cache::new(&text);
        let at = at.unwrap_or_else(|| middle_line_start(&text));
        let mut inserted = false;
        report.check(name, max, || {
            toggle_edit(&mut cache, &mut text, at, insert, &mut inserted);
        });
    }
}

/// The link table's patch after an edit, on top of the cache's own.
fn link_table(report: &mut Report, stress: &str) {
    for (name, insert) in [
        ("link table patch char", "x"),
        ("link table patch newline", "\n"),
    ] {
        let mut text = stress.to_owned();
        let mut links = Links::new(&text);
        let at = middle_line_start(&text);
        let mut inserted = false;
        report.check(name, 64, || {
            let (old_end, new_end) = if inserted {
                text.replace_range(at..at + insert.len(), "");
                (at + insert.len(), at)
            } else {
                text.insert_str(at, insert);
                (at, at + insert.len())
            };
            inserted = !inserted;
            links.edit(&text, at, old_end, new_end);
            reset();
            links.patch(&text);
        });
    }
}

fn fenwick(report: &mut Report, stress: &str) {
    let strides: Vec<i64> = stress
        .split_inclusive('\n')
        .map(|l| l.len() as i64)
        .collect();
    let mut index = Index::new(&strides);
    let mid = strides.len() / 2;
    report.check("fenwick add_at + prefix_sum + find", 0, || {
        index.add_at(mid, 1);
        let sum = index.prefix_sum(mid);
        let _ = index.find(sum);
        index.add_at(mid, -1);
    });
}

/// GPUI allocates for every frame itself (element tree, layout, text
/// shaping), so a frame's bound is a regression ceiling over what is
/// measured today, not a claim that it is small.
fn frames(report: &mut Report, stress: &str) {
    let mut app = HeadlessApp::new(stress);
    app.open_note();
    app.focus_editor();
    let mut typed = false;
    let mut toggle = move || {
        typed = !typed;
        if typed {
            ("x", Some("x"))
        } else {
            ("backspace", None)
        }
    };
    report.check_snapshot("idle frame", 750_000, || app.frame());
    report.check_snapshot("keystroke (edit only)", 16 * 1024, || {
        let (key, text) = toggle();
        app.key(key, text)
    });
    report.check_snapshot("keystroke + frame", 750_000, || {
        let (key, text) = toggle();
        app.key_and_frame(key, text)
    });
    let mut step = 0u32;
    report.check_snapshot("scroll + frame", 1_100_000, || {
        step = (step + 7) % 100;
        app.scroll_and_frame(step as f32 / 100.0)
    });
    let mut dark = false;
    report.check_snapshot("theme switch + frame", 1_100_000, || {
        dark = !dark;
        app.set_theme_and_frame(if dark {
            "catppuccin-mocha"
        } else {
            "catppuccin-latte"
        })
    });
}

fn main() {
    let stress = stress_text();
    let mut report = Report { failures: 0 };
    cache_edits(&mut report, &stress);
    link_table(&mut report, &stress);
    fenwick(&mut report, &stress);
    frames(&mut report, &stress);
    if report.failures > 0 {
        eprintln!(
            "{} operation(s) over their allocation bound",
            report.failures
        );
        std::process::exit(1);
    }
}
