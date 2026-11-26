//! The libraries on their own, without Jade's pre-pass or tree: raw cold
//! parse times, and tree-sitter's own incremental update.

use std::hint::black_box;
use std::time::Instant;

use tree_sitter::{InputEdit, Point, Tree};

use crate::Rng;
use crate::bench::median_ms;
use crate::libs::{Pulldown, TreeSitter};

pub fn cold(text: &str) {
    let report = |name: &str, ms: f64| println!("{name:44} {ms:8.2} ms");
    let options = Pulldown::options();
    report(
        "pulldown: block pass only (Parser::new)",
        median_ms(|| {
            black_box(pulldown_cmark::Parser::new_ext(black_box(text), options));
        }),
    );
    report(
        "pulldown: block + inline (all events)",
        median_ms(|| {
            let parser = pulldown_cmark::Parser::new_ext(black_box(text), options);
            black_box(parser.into_offset_iter().count());
        }),
    );
    let options = comrak::Options::default();
    report(
        "comrak: parse_document (inline included)",
        median_ms(|| {
            let arena = comrak::Arena::new();
            black_box(comrak::parse_document(&arena, black_box(text), &options));
        }),
    );
    let mut block = TreeSitter::new();
    report(
        "tree-sitter-md: block tree only",
        median_ms(|| {
            black_box(block.0.parse(black_box(text), None));
        }),
    );
    let mut both = tree_sitter_md::MarkdownParser::default();
    report(
        "tree-sitter-md: block + inline trees",
        median_ms(|| {
            black_box(both.parse(black_box(text.as_bytes()), None));
        }),
    );
}

fn point(text: &str, at: usize) -> Point {
    let before = &text.as_bytes()[..at];
    let row = before.iter().filter(|&&b| b == b'\n').count();
    let line = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    Point::new(row, at - line)
}

/// Every node as (kind, start, end), in order.
fn nodes(tree: &Tree) -> Vec<(u16, usize, usize)> {
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        out.push((node.kind_id(), node.start_byte(), node.end_byte()));
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

struct Live {
    parser: TreeSitter,
    text: String,
    tree: Option<Tree>,
}

impl Live {
    /// Applies an edit; returns the time of the incremental parse alone and
    /// of `changed_ranges`, which says what to update afterwards.
    fn edit(&mut self, at: usize, removed: usize, inserted: &str) -> (f64, f64) {
        let start_position = point(&self.text, at);
        let old_end_position = point(&self.text, at + removed);
        self.text.replace_range(at..at + removed, inserted);
        let edit = InputEdit {
            start_byte: at,
            old_end_byte: at + removed,
            new_end_byte: at + inserted.len(),
            start_position,
            old_end_position,
            new_end_position: point(&self.text, at + inserted.len()),
        };
        let start = Instant::now();
        let Some(mut old) = self.tree.take() else {
            return (0.0, 0.0);
        };
        old.edit(&edit);
        self.tree = self.parser.0.parse(&self.text, Some(&old));
        let parsed = start.elapsed().as_secs_f64() * 1e3;
        if let Some(new) = &self.tree {
            black_box(old.changed_ranges(new).count());
        }
        let ranged = start.elapsed().as_secs_f64() * 1e3 - parsed;
        (parsed, ranged)
    }
}

/// Is the incremental time the harness's fault? Reparses with an untouched
/// old tree, on the note and on plain paragraphs of the same size.
pub fn sanity(text: &str) {
    let plain = "word word word word.\n\n".repeat(text.len() / 22);
    for (name, source) in [("the note", text), ("plain paragraphs", plain.as_str())] {
        let mut parser = TreeSitter::new();
        let tree = parser.0.parse(source, None);
        let cold = median_ms(|| {
            black_box(parser.0.parse(black_box(source), None));
        });
        let unchanged = median_ms(|| {
            black_box(parser.0.parse(black_box(source), tree.as_ref()));
        });
        println!(
            "ts sanity, {name}: cold {cold:.1} ms, reparse of unchanged text {unchanged:.1} ms"
        );
    }
}

pub fn tree_sitter_incremental(text: &str) {
    let paragraphs: Vec<usize> = text
        .match_indices("\n\n")
        .map(|(i, _)| i)
        .filter(|&i| text[..i].ends_with('.'))
        .collect();
    let scenarios: &[(&str, &[&str], bool)] = &[
        ("char in paragraph", &["e"], false),
        ("Enter in paragraph", &["\n"], false),
        ("``` bare", &["`", "`", "`"], true),
        ("~~~ nothing closes it", &["~", "~", "~"], true),
        ("``` auto-closed", &["`", "`", "`\n\n```"], true),
    ];
    for (name, keys, empty_line) in scenarios {
        let mut live = Live {
            parser: TreeSitter::new(),
            text: text.to_string(),
            tree: None,
        };
        live.tree = live.parser.0.parse(&live.text, None);
        let mut rng = Rng(42);
        let (mut parse, mut ranges) = (Vec::new(), Vec::new());
        let began = Instant::now();
        while parse.len() < 1000 && (began.elapsed().as_secs() < 90 || parse.len() < 60) {
            let mut site = paragraphs[rng.below(paragraphs.len())];
            if !empty_line {
                site -= 1;
            } else {
                live.edit(site, 0, "\n\n");
            }
            let mut at = site + if *empty_line { 2 } else { 0 };
            for key in *keys {
                let (a, b) = live.edit(at, 0, key);
                at += key.len();
                parse.push(a);
                ranges.push(a + b);
            }
            let added = live.text.len() - text.len();
            live.edit(site, added, "");
        }
        // Does the incrementally kept tree equal a fresh one?
        let fresh = TreeSitter::new().0.parse(&live.text, None);
        let same = live.tree.as_ref().map(nodes) == fresh.as_ref().map(nodes);
        for times in [&mut parse, &mut ranges] {
            times.sort_by(f64::total_cmp);
        }
        let at = |t: &[f64], q: usize| t[(t.len() - 1) * q / 100];
        println!(
            "ts native {name:22} parse p50 {:8.3} p99 {:8.3} ms | with changed_ranges p50 {:8.3} p99 {:8.3} ms (n={}){}",
            at(&parse, 50),
            at(&parse, 99),
            at(&ranges, 50),
            at(&ranges, 99),
            parse.len(),
            if same {
                ""
            } else {
                "  TREE DIFFERS FROM FRESH PARSE"
            },
        );
    }
    incremental_fuzz(text);
}

/// Random edits on small texts, checking incremental = full for the native
/// tree.
fn incremental_fuzz(text: &str) {
    const SNIPPETS: &[&str] = &[
        "a",
        " ",
        "\n",
        "\n\n",
        "```",
        "```\n",
        "~~~\n",
        "- ",
        "1. ",
        "> ",
        "# ",
        "|",
        "| a | b |\n|---|---|\n",
        "---",
        "===\n",
        "    ",
        "[r]: /url\n",
    ];
    let (mut differing, rounds) = (0, 200);
    for round in 0..rounds {
        let mut rng = Rng(round * 7919 + 1);
        let mut from = rng.below(text.len() - 3000);
        while !text.is_char_boundary(from) {
            from -= 1;
        }
        let mut slice: String = text[from..].chars().take(1500).collect();
        slice.push('\n');
        let mut live = Live {
            parser: TreeSitter::new(),
            text: slice,
            tree: None,
        };
        live.tree = live.parser.0.parse(&live.text, None);
        for _ in 0..100 {
            let mut at = rng.below(live.text.len());
            while !live.text.is_char_boundary(at) {
                at -= 1;
            }
            let delete = rng.below(4) == 0;
            let mut end = (at + 1 + rng.below(30)).min(live.text.len());
            while !live.text.is_char_boundary(end) {
                end -= 1;
            }
            let removed = if delete { end.saturating_sub(at) } else { 0 };
            let inserted = if delete {
                ""
            } else {
                SNIPPETS[rng.below(SNIPPETS.len())]
            };
            live.edit(at, removed, inserted);
            let fresh = TreeSitter::new().0.parse(&live.text, None);
            if live.tree.as_ref().map(nodes) != fresh.as_ref().map(nodes) {
                differing += 1;
                live.tree = fresh;
            }
        }
    }
    println!(
        "ts native random edits: tree differs from a fresh parse after {differing}/{} edits",
        rounds * 100
    );
}
