use criterion::{Criterion, criterion_group, criterion_main};
use markdown::ParseOptions;

/// A typical note is ~500-2000 words; a worst-case one (e.g. an imported book
/// chapter or meeting-notes dump) is ~20k words. Headings every 40 words and
/// a `[[Wikilink]]` every 20 words keep this vaguely note-shaped rather than
/// one giant paragraph of plain prose, since wikilinks are plain text to this
/// parser (no CommonMark/GFM construct matches `[[...]]`) but still add
/// bracket/text churn a real note would have.
fn synthetic_note(word_count: usize) -> String {
    let mut text = String::with_capacity(word_count * 6);
    for i in 0..word_count {
        if i % 40 == 0 {
            text.push_str(&format!("\n\n## Section {i}\n\n"));
        }
        if i > 0 && i % 20 == 0 {
            text.push_str(&format!("[[Note {}]] ", i % 500));
        } else {
            text.push_str("word ");
        }
    }
    text
}

/// Baseline parse cost of `markdown::to_mdast` alone, with the same `gfm()`
/// options `gpui-component`'s `TextView::markdown` uses internally — this is
/// the floor our own block-segmentation logic (not yet written) will add on
/// top of, so it's worth tracking before that code exists, not just after.
fn bench_to_mdast(c: &mut Criterion) {
    let options = ParseOptions::gfm();
    let mut group = c.benchmark_group("markdown::to_mdast");
    for &(label, words) in &[("typical_note", 1_500usize), ("worst_case_note", 20_000usize)] {
        let source = synthetic_note(words);
        group.bench_function(label, |b| {
            b.iter(|| markdown::to_mdast(&source, &options).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, bench_to_mdast);
criterion_main!(benches);
