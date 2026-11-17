use criterion::{Criterion, criterion_group, criterion_main};
use jade_core::{Vault, extract_wikilinks};
use tempfile::TempDir;

/// A typical note is ~500-2000 words; a worst-case one (e.g. an imported
/// book chapter or meeting-notes dump) is ~20k words. Headings every 40
/// words and a `[[Wikilink]]` every `link_every` words keep this vaguely
/// note-shaped rather than one giant paragraph of plain prose.
fn synthetic_note(word_count: usize, link_every: usize) -> String {
    let mut text = String::with_capacity(word_count * 6);
    for i in 0..word_count {
        if i % 40 == 0 {
            text.push_str(&format!("\n\n## Section {i}\n\n"));
        }
        if link_every > 0 && i > 0 && i % link_every == 0 {
            text.push_str(&format!("[[Note {}]] ", i % 500));
        } else {
            text.push_str("word ");
        }
    }
    text
}

/// Goes through the real `create_note`/`save_note` API (backed by a
/// real temp directory) rather than constructing `Vault`'s fields directly, so
/// this also exercises the real incremental wikilink-cache updates, not just a
/// hand-assembled fixture. The `TempDir` must stay alive as long as `Vault`
/// does: `search` reads the notes back from disk.
fn vault_with_notes(count: usize, words_per_note: usize) -> (TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    for i in 0..count {
        let id = vault.create_note(&format!("Note {i}"), None).unwrap();
        vault
            .save_note(&id, &synthetic_note(words_per_note, 20))
            .unwrap();
    }
    (dir, vault)
}

const SIZES: &[(&str, usize)] = &[("typical", 1_500), ("worst_case", 20_000)];

fn bench_extract_wikilinks(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_wikilinks");
    for &(label, words) in SIZES {
        let note = synthetic_note(words, 20);
        group.bench_function(label, |b| b.iter(|| extract_wikilinks(&note)));
    }
    group.finish();
}

/// Opening a vault scans every note line by line for wikilinks, so one stress
/// note (11.8MB, 320K lines, 27K links) times that scan; autosave rescans the
/// same way. Skipped when the fixture is absent.
fn bench_open_stress_note(c: &mut Criterion) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../app/benches/fixtures/large_markdown_stress_test.md"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Stress.md"), text).unwrap();
    let mut group = c.benchmark_group("Vault::open");
    group.sample_size(20);
    group.bench_function("stress_note", |b| {
        b.iter(|| Vault::open(dir.path()).unwrap());
    });
    group.finish();
}

/// 200 notes is a modest-but-real vault size; `backlinks()`/`search()` are
/// both full scans over every note, so this is where vault-wide operations
/// would first start to show up as sluggish.
fn bench_backlinks(c: &mut Criterion) {
    let mut group = c.benchmark_group("Vault::backlinks");
    for &(label, words) in SIZES {
        let (_dir, vault) = vault_with_notes(200, words);
        group.bench_function(label, |b| b.iter(|| vault.build_backlinks()));
    }
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("Vault::search");
    for &(label, words) in SIZES {
        let (_dir, vault) = vault_with_notes(200, words);
        group.bench_function(label, |b| b.iter(|| vault.search("word")));
        // No note matches, so every byte of every file is scanned: the
        // real worst case (a match exits early).
        group.bench_function(format!("{label}_no_match"), |b| {
            b.iter(|| vault.search("zzzqqq"))
        });
    }
    group.finish();
}

/// The part of a background search that still runs on the UI thread.
fn bench_search_snapshot(c: &mut Criterion) {
    let (_dir, vault) = vault_with_notes(200, 100);
    c.bench_function("Vault::search_targets/200_notes", |b| {
        b.iter(|| vault.search_targets())
    });
}

/// A rename reads every note once and rewrites each one linking to the
/// renamed note; the synthetic notes all link to `Note 0`, so every one does.
fn bench_rename(c: &mut Criterion) {
    let mut group = c.benchmark_group("Vault::rename_note_updating_links");
    group.sample_size(10);
    for &(label, words) in SIZES {
        let (_dir, mut vault) = vault_with_notes(200, words);
        let mut name = "Note 0".to_string();
        let mut flip = false;
        group.bench_function(format!("200_notes/{label}"), |b| {
            b.iter(|| {
                flip = !flip;
                let next = if flip { "Renamed" } else { "Note 0" };
                let done = vault.rename_note_updating_links(&name, next).unwrap();
                name = done.new_id;
                done.rewritten.len()
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_extract_wikilinks,
    bench_backlinks,
    bench_search,
    bench_search_snapshot,
    bench_open_stress_note,
    bench_rename
);
criterion_main!(benches);
