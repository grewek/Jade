//! Correctness checks: exact ranges (criterion 5), incremental = full,
//! extensions and error locality (3, 4), and the CommonMark block structure.

use std::fmt::Write;

use crate::Rng;
use crate::backends::Candidate;
use crate::tree::{self, Edit, Kind, Unit};

/// Criterion 5: units in order and disjoint, nodes inside their unit, and
/// nothing but white space left uncovered.
pub fn ranges(text: &str, units: &[Unit]) -> String {
    let (mut covered, mut disorder, mut stray) = (0, 0, 0);
    let mut uncovered = Vec::new();
    for unit in units {
        let start = unit.start as usize;
        if start < covered || unit.end() > text.len() {
            disorder += 1;
        } else if !text[covered..start].trim().is_empty() {
            uncovered.push(text[covered..start].trim());
        }
        covered = covered.max(unit.end());
        let outside = |n: &&tree::Node| n.start + n.len > unit.len;
        stray += unit.children.iter().filter(outside).count();
    }
    if !text[covered.min(text.len())..].trim().is_empty() {
        uncovered.push(text[covered..].trim());
    }
    let bytes: usize = uncovered.iter().map(|s| s.len()).sum();
    let example: String = uncovered
        .first()
        .map_or(String::new(), |s| s.chars().take(60).collect());
    format!(
        "{} units, {disorder} out of order, {stray} stray nodes, {} uncovered spans ({bytes} B) {example:?}",
        units.len(),
        uncovered.len()
    )
}

const SNIPPETS: &[&str] = &[
    "a",
    " ",
    "x y",
    "\n",
    "\n",
    "\n\n",
    "```",
    "```\n",
    "~~~\n",
    "$$",
    "$$\n",
    "- ",
    "* ",
    "1. ",
    "> ",
    ">",
    "# ",
    "|",
    "| a | b |\n|---|---|\n",
    "|---|\n",
    "---",
    "===\n",
    "    ",
    "  ",
    "</jade-aside>\n",
    "<jade-aside side=\"left\">\n",
    "<jade-columns>\n",
    "<jade-column>\n",
    "</jade-column>\n",
    "</jade-columns>\n",
    "[!note] ",
    "[r]: /url\n",
    "![a](b.png)",
    "`",
];

fn char_floor(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Random edits, each checked against a fresh parse. Returns the number of
/// mismatches and a description of the first one.
pub fn fuzz(candidate: &Candidate, seed_text: &str, rounds: usize) -> (usize, String) {
    let mut parser = candidate.1();
    let (mut mismatches, mut first) = (0, String::new());
    let mut fresh = Vec::new();
    for round in 0..rounds {
        let mut rng = Rng(round as u64 * 7919 + 1);
        let from = char_floor(seed_text, rng.below(seed_text.len() - 3000));
        let from = tree::line_at(seed_text, from).1;
        let mut text = seed_text[from..char_floor(seed_text, from + 1500)].to_string();
        let mut units = parser.parse(&text);
        for step in 0..100 {
            let at = char_floor(&text, rng.below(text.len() + 1));
            // Half of the edits at a line start, where block syntax lives.
            let at = match rng.below(2) {
                0 => text[..at].rfind('\n').map_or(0, |i| i + 1),
                _ => at,
            };
            let delete = rng.below(4) == 0;
            let end = char_floor(&text, (at + 1 + rng.below(30)).min(text.len()));
            let removed = if delete { end.saturating_sub(at) } else { 0 };
            let inserted = if delete {
                ""
            } else {
                SNIPPETS[rng.below(SNIPPETS.len())]
            };
            let before = text.clone();
            text.replace_range(at..at + removed, inserted);
            let edit = Edit {
                at,
                removed,
                inserted: inserted.len(),
            };
            let old = units.clone();
            tree::update(parser.as_mut(), &mut units, &mut fresh, &text, edit);
            let full = parser.parse(&text);
            if units != full {
                if first.is_empty() {
                    let diff = units.iter().zip(&full).position(|(a, b)| a != b);
                    if std::env::var_os("SPIKE_DEBUG").is_some() {
                        for (name, source, tree) in [
                            ("old", &before, &old),
                            ("incremental", &text, &units),
                            ("full", &text, &full),
                        ] {
                            eprintln!("-- {name}");
                            for u in tree.iter().take(diff.unwrap_or(0) + 5) {
                                let body = &source[u.start as usize..u.end()];
                                eprintln!("{} {:?} {:?} {body:?}", u.start, u.kind, u.ctx);
                            }
                        }
                    }
                    let _ = write!(
                        first,
                        "round {round} step {step}: {edit:?} ins {inserted:?} first diff unit {diff:?}\n\
                         incremental {:?}\nfull {:?}\nbefore {before:?}",
                        diff.and_then(|i| units.get(i)),
                        diff.and_then(|i| full.get(i)),
                    );
                }
                mismatches += 1;
                units = full;
            }
        }
    }
    (mismatches, first)
}

pub const CASES: &[(&str, &str)] = &[
    (
        "aside with table",
        "<jade-aside side=\"left\">\n\n| a |\n|---|\n| 1 |\n\ntext\n\n</jade-aside>\n\nafter\n",
    ),
    (
        "closer after paragraph",
        "<jade-aside side=\"left\">\n\ntext\n</jade-aside>\nafter\n",
    ),
    (
        "columns",
        "<jade-columns>\n\n<jade-column>\n\nl\n\n</jade-column>\n<jade-column>\n\nr\n\n</jade-column>\n\n</jade-columns>\n",
    ),
    (
        "open fence in aside",
        "<jade-aside side=\"left\">\n\n```\ncode\n\n</jade-aside>\n\n# after\n",
    ),
    (
        "open $$ in aside",
        "<jade-aside side=\"left\">\n\n$$\nx\n</jade-aside>\n\n# after\n",
    ),
    (
        "tag inside fence",
        "```\n<jade-aside side=\"left\">\n```\ntext\n",
    ),
    ("$$ block", "$$\na = b\n$$\n"),
    ("$$ with list-like line", "$$\n- x\n$$\n"),
    ("$$ with setext line", "$$\na\n=\nb\n$$\n"),
    ("$$ after text", "text\n$$\n# x\n$$\n"),
    ("unclosed $$", "$$\nx\n\n# heading\n"),
    ("callout", "> [!warning]- Title\n> body\n> - item\n"),
    ("wikilink pipe in table", "| a |\n|---|\n| [[x\\|y]] |\n"),
    ("html comment line", "<div>\n**x**\n- a\n"),
];

pub fn outline(text: &str, units: &[Unit]) -> String {
    let mut out = String::new();
    for unit in units {
        let first = text[unit.start as usize..unit.end()]
            .lines()
            .next()
            .unwrap_or("");
        let _ = write!(out, "{:?}({:?}", unit.kind, first);
        for node in &unit.children {
            let _ = write!(out, " {}{:?}", node.depth, node.kind);
        }
        out.push_str(") ");
    }
    out
}

/// A tree as the block elements CommonMark's HTML would have: depth and tag.
fn skeleton(text: &str, units: &[Unit]) -> Vec<String> {
    let mut out = Vec::new();
    for unit in units {
        let mut items: Vec<u8> = Vec::new();
        let root = tree::Node {
            kind: unit.kind,
            depth: 0,
            start: 0,
            len: unit.len,
        };
        for node in std::iter::once(&root).chain(&unit.children) {
            items.retain(|&depth| depth < node.depth);
            let source = &text[(unit.start + node.start) as usize..];
            let tag = match node.kind {
                // A tight list's item text has no <p>; neither side counts it.
                Kind::Paragraph | Kind::Image | Kind::Math if !items.is_empty() => continue,
                Kind::Paragraph | Kind::Image | Kind::Math => "p",
                Kind::Heading => "h",
                Kind::List => list_tag(source),
                Kind::Item => "li",
                Kind::Quote | Kind::Callout => "blockquote",
                Kind::Code => "pre",
                Kind::Rule => "hr",
                Kind::RefDef => continue,
                _ => "other",
            };
            if node.kind == Kind::Item {
                items.push(node.depth);
            }
            out.push(format!("{}{tag}", node.depth));
        }
    }
    out
}

fn list_tag(source: &str) -> &'static str {
    match source.trim_start().bytes().next() {
        Some(b'0'..=b'9') => "ol",
        _ => "ul",
    }
}

/// `dir` holds `N.md` and `N.expected` (one `depth+tag` per line).
pub fn spec(candidate: &Candidate, dir: &std::path::Path) -> std::io::Result<String> {
    let index = std::fs::read_to_string(dir.join("index.tsv"))?;
    let mut parser = candidate.1();
    let (mut total, mut passed) = (0, 0);
    let mut failures = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for row in index.lines() {
        let Some((number, section)) = row.split_once('\t') else {
            continue;
        };
        let text = std::fs::read_to_string(dir.join(format!("{number}.md")))?;
        let expected = std::fs::read_to_string(dir.join(format!("{number}.expected")))?;
        let got = skeleton(&text, &parser.parse(&text));
        total += 1;
        if got == expected.lines().collect::<Vec<_>>() {
            passed += 1;
        } else {
            failures.entry(section).or_default().push(number);
        }
    }
    let mut out = format!("block structure matches CommonMark in {passed}/{total} examples\n");
    for (section, numbers) in failures {
        let _ = writeln!(out, "  {section}: {} fail {:?}", numbers.len(), numbers);
    }
    Ok(out)
}
