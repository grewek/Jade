//! Where the links of the open note are. Only offsets are stored (16 bytes a
//! link: line, byte range within the line, kind); the target, heading and
//! alias are read back out of the line by re-parsing its span with
//! [`jade_core::link_scan`], so the table never holds note text.
//!
//! Sparse and sorted by `(line, start)`: lines without links cost nothing and
//! a lookup is a binary search. Kept in step with the text like `DocStats`:
//! an edit rescans only the lines `EditPatch::touched` names and shifts the
//! later entries' line numbers when the line count changed.
//!
//! Only `Plain` lines are scanned: fenced code is never Markdown. Links the
//! scanner finds inside bold or italic text are stored too, although Live
//! Preview does not paint them as links yet.

use std::ops::Range;

use jade_core::link_scan::{LinkKind, scan_line};

use super::block_preview::LineRole;
use super::doc_stats::{MAX_THREADS, PARALLEL_MIN_BYTES};
use super::document_cache::{DocumentCache, EditPatch};

/// Room kept free after a rebuild so typing a link does not reallocate.
const SLACK: usize = 64;

/// Whether a link's target exists. Set by [`LinkTable::resolve`], which the
/// app runs against the vault; everything starts `Unchecked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkState {
    Unchecked,
    Resolved,
    Broken,
}

/// One link: `start..end` is its whole span (delimiters included) in bytes
/// from the start of line `line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StoredLink {
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub kind: LinkKind,
    pub state: LinkState,
}

pub(super) struct LinkTable {
    links: Vec<StoredLink>,
    /// Reused by `patch` to hold a rescan before it is spliced in.
    scratch: Vec<StoredLink>,
}

/// Appends the links of `line` (text of line number `number`) to `out`.
fn scan_into(out: &mut Vec<StoredLink>, number: u32, line: &str) {
    scan_line(line, |link| {
        // `line.len()` fits in a `u32`: tables are only built for text that does.
        out.push(StoredLink {
            line: number,
            start: link.span.start as u32,
            end: link.span.end as u32,
            kind: link.kind,
            // A URL on the web is not the vault's to check.
            state: match link.kind {
                LinkKind::Wiki | LinkKind::Inline => LinkState::Unchecked,
                LinkKind::Autolink | LinkKind::BareUrl => LinkState::Resolved,
            },
        });
    });
}

/// Scans the lines of `chunk`, the first being document line `first`, into `out`.
fn scan_chunk(chunk: &str, first: usize, roles: &[LineRole], out: &mut Vec<StoredLink>) {
    for (offset, line) in chunk.split('\n').enumerate() {
        let number = first + offset;
        if roles.get(number) == Some(&LineRole::Plain) {
            scan_into(out, number as u32, line);
        }
    }
}

/// Notes bigger than this get no table; offsets are `u32`.
fn fits(text: &str) -> bool {
    text.len() <= u32::MAX as usize
}

impl LinkTable {
    pub(super) fn new(text: &str, doc: &DocumentCache) -> Self {
        let mut table = Self {
            links: Vec::new(),
            scratch: Vec::new(),
        };
        table.rebuild(text, doc);
        table
    }

    /// Scans the whole document: the cold open, and the fallback when an edit
    /// could not be patched. A big note is scanned by several threads, each
    /// over a run of whole lines (about 22ms single-threaded on the 11.8MB
    /// stress note).
    pub(super) fn rebuild(&mut self, text: &str, doc: &DocumentCache) {
        self.links.clear();
        if !fits(text) {
            return;
        }
        let roles = doc.line_roles();
        let threads = if text.len() < PARALLEL_MIN_BYTES {
            1
        } else {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS))
        };
        let mut cuts = vec![0];
        for k in 1..threads {
            let guess = text.len() / threads * k;
            let cut = text[guess..]
                .find('\n')
                .map_or(text.len(), |i| guess + i + 1);
            cuts.push(cut.max(cuts[k - 1]));
        }
        cuts.push(text.len());
        // Each chunk starts on a line boundary, so the line index knows its
        // first line number without counting newlines.
        let chunks: Vec<(&str, usize)> = cuts
            .windows(2)
            .map(|w| {
                (
                    &text[w[0]..w[1]],
                    if w[0] == 0 { 0 } else { doc.line_at(w[0]) },
                )
            })
            .collect();
        if let [(chunk, first)] = chunks[..] {
            scan_chunk(chunk, first, roles, &mut self.links);
        } else {
            let parts: Vec<Vec<StoredLink>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunks
                    .iter()
                    .map(|&(chunk, first)| {
                        scope.spawn(move || {
                            let mut part = Vec::new();
                            scan_chunk(chunk, first, roles, &mut part);
                            part
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join()
                            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                    })
                    .collect()
            });
            self.links.reserve_exact(parts.iter().map(Vec::len).sum());
            for part in parts {
                self.links.extend(part);
            }
        }
        // Growth by doubling leaves up to half the vector unused.
        self.links.shrink_to_fit();
        // `reserve` would double a full vector.
        self.links.reserve_exact(SLACK);
    }

    /// Rescans the lines `patch` touched; `text` and `doc` already reflect the edit.
    pub(super) fn patch(&mut self, text: &str, doc: &DocumentCache, patch: &EditPatch) {
        if !fits(text) {
            self.links.clear();
            return;
        }
        // Lines after the edit moved by `line_delta`, so the touched span's
        // old extent ends that much earlier.
        let start = patch.touched.start as u32;
        let old_end = (patch.touched.end as isize - patch.line_delta) as u32;
        let new_end = patch.touched.end as u32;
        let roles = doc.line_roles();

        self.scratch.clear();
        for number in start..new_end {
            if roles.get(number as usize) == Some(&LineRole::Plain) {
                let line =
                    &text[doc.line_start_byte(number as usize)..doc.line_end_byte(number as usize)];
                scan_into(&mut self.scratch, number, line);
            }
        }

        let first = self.links.partition_point(|l| l.line < start);
        let last = self.links.partition_point(|l| l.line < old_end);
        let fresh = self.scratch.len();
        self.links.splice(first..last, self.scratch.drain(..));
        if patch.line_delta != 0 {
            for link in &mut self.links[first + fresh..] {
                link.line = (i64::from(link.line) + patch.line_delta as i64) as u32;
            }
        }
    }

    /// Forgets which links resolved, after the vault's notes changed.
    pub(super) fn invalidate(&mut self) {
        for link in &mut self.links {
            if matches!(link.kind, LinkKind::Wiki | LinkKind::Inline) {
                link.state = LinkState::Unchecked;
            }
        }
    }

    /// Settles every `Unchecked` link: `is_live(kind, dest)` says whether its
    /// target exists. Only links scanned since the last call are asked, so a
    /// keystroke costs a lookup for the links on its own line. Returns whether
    /// any of them came out broken.
    pub(super) fn resolve(
        &mut self,
        text: &str,
        doc: &DocumentCache,
        mut is_live: impl FnMut(LinkKind, &str) -> bool,
    ) -> bool {
        let mut any_broken = false;
        for i in 0..self.links.len() {
            let stored = self.links[i];
            if stored.state != LinkState::Unchecked {
                continue;
            }
            let line = stored.line as usize;
            let line_text = &text[doc.line_start_byte(line)..doc.line_end_byte(line)];
            let mut live = true;
            scan_line(line_text, |link| {
                if link.span.start == stored.start as usize && link.kind == stored.kind {
                    live = is_live(link.kind, &line_text[link.dest]);
                }
            });
            let state = if live {
                LinkState::Resolved
            } else {
                LinkState::Broken
            };
            any_broken |= state == LinkState::Broken;
            self.links[i].state = state;
        }
        any_broken
    }

    /// How many links are known to point at nothing.
    #[cfg_attr(
        not(any(test, feature = "debug-socket")),
        allow(dead_code, reason = "read by the harness's `app` query")
    )]
    pub(super) fn broken_count(&self) -> usize {
        self.links
            .iter()
            .filter(|l| l.state == LinkState::Broken)
            .count()
    }

    /// The byte ranges within `line` of its links whose target is missing.
    pub(super) fn broken_on_line(&self, line: usize) -> impl Iterator<Item = Range<usize>> + '_ {
        self.on_line(line)
            .iter()
            .filter(|l| l.state == LinkState::Broken)
            .map(|l| l.start as usize..l.end as usize)
    }

    #[cfg_attr(
        not(any(test, feature = "debug-socket")),
        allow(dead_code, reason = "read by the harness's `app` query")
    )]
    pub(super) fn len(&self) -> usize {
        self.links.len()
    }

    /// The links on `line`, left to right.
    pub(super) fn on_line(&self, line: usize) -> &[StoredLink] {
        let line = line as u32;
        let first = self.links.partition_point(|l| l.line < line);
        let last = self.links.partition_point(|l| l.line <= line);
        &self.links[first..last]
    }

    /// The link on `line` whose span covers byte `column` of that line.
    pub(super) fn at(&self, line: usize, column: usize) -> Option<&StoredLink> {
        self.on_line(line)
            .iter()
            .find(|l| (l.start as usize..l.end as usize).contains(&column))
    }

    /// Bytes the table holds (capacity, which is what is retained).
    #[cfg(test)]
    pub(super) fn heap_bytes(&self) -> usize {
        (self.links.capacity() + self.scratch.capacity()) * std::mem::size_of::<StoredLink>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_editor::document_cache::TextEdit;

    fn table_of(text: &str) -> LinkTable {
        LinkTable::new(text, &DocumentCache::new(text))
    }

    fn spans(table: &LinkTable) -> Vec<(u32, u32, u32, LinkKind)> {
        table
            .links
            .iter()
            .map(|l| (l.line, l.start, l.end, l.kind))
            .collect()
    }

    #[test]
    fn a_stored_link_is_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<StoredLink>(), 16);
    }

    #[test]
    fn finds_every_kind_with_line_and_column() {
        let text = "intro [[a]]\nno links\n- [t](u.md) and <https://x.y>\nhttps://z.w";
        let table = table_of(text);
        assert_eq!(
            spans(&table),
            [
                (0, 6, 11, LinkKind::Wiki),
                (2, 2, 11, LinkKind::Inline),
                (2, 16, 29, LinkKind::Autolink),
                (3, 0, 11, LinkKind::BareUrl),
            ]
        );
        assert_eq!(table.on_line(1), []);
        assert_eq!(table.on_line(2).len(), 2);
        assert_eq!(table.at(0, 6).map(|l| l.kind), Some(LinkKind::Wiki));
        assert_eq!(table.at(0, 10).map(|l| l.kind), Some(LinkKind::Wiki));
        assert_eq!(table.at(0, 11), None, "the end is exclusive");
        assert_eq!(table.at(0, 5), None);
        assert_eq!(table.at(9, 0), None);
    }

    /// A table over `text` whose wikilink and path-link targets exist only if
    /// they contain "ok".
    fn resolved(text: &str) -> (LinkTable, DocumentCache) {
        let cache = DocumentCache::new(text);
        let mut table = LinkTable::new(text, &cache);
        table.resolve(text, &cache, |_, dest| dest.contains("ok"));
        (table, cache)
    }

    fn states(table: &LinkTable) -> Vec<LinkState> {
        table.links.iter().map(|l| l.state).collect()
    }

    #[test]
    fn resolve_marks_missing_targets_and_leaves_web_links_alone() {
        let text = "[[ok]] [[gone]] [a](ok.md) [b](gone.md) <https://x.y> https://z.w";
        let (table, _) = resolved(text);
        use LinkState::{Broken, Resolved};
        assert_eq!(
            states(&table),
            [Resolved, Broken, Resolved, Broken, Resolved, Resolved]
        );
        assert_eq!(table.broken_count(), 2);
        assert_eq!(table.broken_on_line(0).collect::<Vec<_>>(), [7..15, 27..39]);
    }

    #[test]
    fn an_edit_unchecks_only_the_links_it_rescanned() {
        let text = "[[gone]]\n[[ok]]\nplain";
        let mut cache = DocumentCache::new(text);
        let mut table = LinkTable::new(text, &cache);
        table.resolve(text, &cache, |_, dest| dest.contains("ok"));
        let new_text = "[[gone]]\n[[ok]] x\nplain";
        let edit = TextEdit {
            start_byte: 15,
            old_end_byte: 15,
            new_end_byte: 18,
        };
        let patch = cache.record_edit(new_text, edit).unwrap();
        table.patch(new_text, &cache, &patch);
        assert_eq!(states(&table), [LinkState::Broken, LinkState::Unchecked]);
        table.resolve(new_text, &cache, |_, _| true);
        assert_eq!(states(&table), [LinkState::Broken, LinkState::Resolved]);
    }

    #[test]
    fn invalidate_unchecks_vault_links_but_not_web_ones() {
        let (mut table, cache) = resolved("[[gone]] <https://x.y>");
        table.invalidate();
        assert_eq!(states(&table), [LinkState::Unchecked, LinkState::Resolved]);
        table.resolve("[[gone]] <https://x.y>", &cache, |_, _| true);
        assert_eq!(table.broken_count(), 0);
    }

    #[test]
    fn fenced_code_is_not_scanned() {
        let text = "[[a]]\n```\n[[b]] [t](u)\n```\n[[c]]";
        let lines: Vec<u32> = table_of(text).links.iter().map(|l| l.line).collect();
        assert_eq!(lines, [0, 4]);
    }

    #[test]
    fn empty_and_link_free_notes_have_an_empty_table() {
        assert_eq!(table_of("").len(), 0);
        assert_eq!(table_of("just\nplain text\n").len(), 0);
    }

    /// The table must equal a fresh build after every edit: line-count
    /// changes, fences opening and closing, joins, and the first and last line.
    #[test]
    fn patched_table_matches_a_fresh_one_after_random_edits() {
        let alphabet: Vec<&str> = vec![
            "a",
            " ",
            "\n",
            "[[",
            "]]",
            "[",
            "](",
            ")",
            "<",
            ">",
            "`",
            "```\n",
            "http://x.y",
            "|",
            "\u{e9}",
            "- ",
            "> ",
            "#",
            "**",
            "\\",
        ];
        for seed in [0x2545_f491_4f6c_dd1d_u64, 7, 0xdead_beef_cafe_f00d] {
            let mut state = seed;
            let mut next = move |n: usize| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % n as u64) as usize
            };
            let mut text =
                String::from("start [[a]]\n\n- [t](u)\n```\n[[in fence]]\n```\nend <https://x>");
            let mut cache = DocumentCache::new(&text);
            let mut table = LinkTable::new(&text, &cache);
            for step in 0..600 {
                let starts: Vec<usize> = text
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain([text.len()])
                    .collect();
                let a = starts[next(starts.len())];
                let b = starts[next(starts.len())];
                let (start, far) = (a.min(b), a.max(b).min(a.min(b) + 12));
                let old_end = starts.iter().copied().filter(|&c| c <= far).max().unwrap();
                let insert: String = (0..next(4))
                    .map(|_| alphabet[next(alphabet.len())])
                    .collect();
                let new_text = format!("{}{insert}{}", &text[..start], &text[old_end..]);
                let edit = TextEdit {
                    start_byte: start,
                    old_end_byte: old_end,
                    new_end_byte: start + insert.len(),
                };
                match cache.record_edit(&new_text, edit) {
                    Some(patch) => table.patch(&new_text, &cache, &patch),
                    None => table.rebuild(&new_text, &cache),
                }
                text = new_text;
                let fresh = LinkTable::new(&text, &cache);
                assert_eq!(
                    spans(&table),
                    spans(&fresh),
                    "seed {seed:#x} step {step} text {text:?}"
                );
            }
        }
    }

    /// The threaded cold scan must equal a plain line-by-line one, wherever
    /// the chunk cuts fall: inside a fence, in blank runs, in one huge line.
    #[test]
    fn threaded_scan_matches_a_sequential_one() {
        let unit = "text [[a]] more\n\n```\n[[fenced]]\n```\n- [t](u.md) <https://x.y>\n\n\nhttps://z.w end";
        let long_line = "x [[b]] ".repeat(PARALLEL_MIN_BYTES / 8 + 1);
        let texts = [
            unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2),
            format!("{}\n", unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2)),
            format!(
                "```\n{}",
                "[[in an open fence]]\n".repeat(PARALLEL_MIN_BYTES / 20 + 2)
            ),
            "\n".repeat(PARALLEL_MIN_BYTES + 5),
            format!("start [[a]]\n{long_line}\nend [[c]]"),
            long_line,
        ];
        for text in &texts {
            assert!(text.len() >= PARALLEL_MIN_BYTES);
            let cache = DocumentCache::new(text);
            let mut expected = Vec::new();
            for (number, line) in text.split('\n').enumerate() {
                if cache.line_roles().get(number) == Some(&LineRole::Plain) {
                    scan_into(&mut expected, number as u32, line);
                }
            }
            let table = LinkTable::new(text, &cache);
            assert_eq!(table.links, expected, "{}", &text[..text.len().min(40)]);
        }
    }

    #[test]
    fn the_stress_fixture_table_is_small() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/benches/fixtures/large_markdown_stress_test.md"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let cache = DocumentCache::new(&text);
        let table = LinkTable::new(&text, &cache);
        // The fixture's 27K `[[not a link]]` are all inside code fences; only
        // its real links are stored.
        assert!(table.len() > 100, "{}", table.len());
        assert!(table.heap_bytes() < 256 * 1024, "{}", table.heap_bytes());
    }

    /// A note made of nothing but links is the table's worst case: 16 bytes a
    /// link, against ~51 bytes a line for the classification arrays.
    #[test]
    fn a_hundred_thousand_links_cost_under_two_megabytes() {
        let text = "[[a]] [t](u) <https://x.y>\n".repeat(34_000);
        let cache = DocumentCache::new(&text);
        let table = LinkTable::new(&text, &cache);
        assert_eq!(table.len(), 102_000);
        assert!(
            table.heap_bytes() < 2 * 1024 * 1024,
            "{}",
            table.heap_bytes()
        );
    }
}
