//! The lines around the links that point at a note, for the backlinks panel.
//! Read from the source notes' files on demand with the shared scanner, so no
//! context is kept in memory between calls.

use crate::fence::plain_lines;
use crate::link_scan::scan_line;
use crate::resolve::Destination;
use crate::vault::Vault;

/// How far a snippet reaches on each side of its link, in bytes.
const REACH: usize = 60;

/// One line of a source note that links to the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkContext {
    /// Zero-based line number in the source note.
    pub line: usize,
    /// The line, trimmed and cut to a window around its first link to the
    /// target, with `…` where text was cut.
    pub snippet: String,
}

/// What one source note contributes to the target's backlinks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceContexts {
    pub source: String,
    /// The first lines that link to the target, at most `limit`.
    pub contexts: Vec<LinkContext>,
    /// How many lines link to it in all.
    pub total: usize,
}

impl Vault {
    /// For each of `sources`, the lines that link to note `target` (wikilinks
    /// and `[text](path)` links, outside fenced code), in note order. Reads
    /// each source's file; a source that cannot be read, or has no such line,
    /// is left out. Call it when the panel's inputs change, not per frame.
    pub fn link_contexts(
        &self,
        target: &str,
        sources: &[String],
        limit: usize,
    ) -> Vec<SourceContexts> {
        let mut text = String::new();
        let mut found = Vec::new();
        for source in sources {
            let Some(note) = self.notes.get(source) else {
                continue;
            };
            if note.read_content_into(&mut text).is_err() {
                continue;
            }
            let mut contexts = Vec::new();
            let mut total = 0;
            for (n, line) in plain_lines(&text) {
                if !(line.contains("[[") || line.contains("](")) {
                    continue;
                }
                let mut first = None;
                scan_line(line, |link| {
                    if first.is_some() {
                        return;
                    }
                    let written = &line[link.dest.clone()];
                    let hit = matches!(
                        self.link_destination(source, link.kind, written),
                        Destination::Note(id) if id == target
                    );
                    if hit {
                        first = Some(link.span);
                    }
                });
                let Some(span) = first else {
                    continue;
                };
                total += 1;
                if contexts.len() < limit {
                    contexts.push(LinkContext {
                        line: n,
                        snippet: snippet(line, span),
                    });
                }
            }
            if total > 0 {
                found.push(SourceContexts {
                    source: source.clone(),
                    contexts,
                    total,
                });
            }
        }
        found
    }
}

/// `line` trimmed and cut to `REACH` bytes either side of `span`.
fn snippet(line: &str, span: std::ops::Range<usize>) -> String {
    let line_start = line.len() - line.trim_start().len();
    let line_end = line.trim_end().len();
    let mut start = span.start.saturating_sub(REACH).max(line_start);
    let mut end = (span.end + REACH).min(line_end);
    while !line.is_char_boundary(start) {
        start += 1;
    }
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::new();
    if start > line_start {
        out.push('…');
    }
    out.push_str(line[start..end].trim());
    if end < line_end {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {

    use crate::test_support::vault_with as vault;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_the_lines_that_link_with_their_numbers() {
        let (_d, v) = vault(&[
            ("T.md", ""),
            (
                "A.md",
                "intro\nsee [[T]] here\n\n- also [x](T.md#h)\nnot [[Other]]\n[[t|again]] and [[T]] twice",
            ),
        ]);
        let found = v.link_contexts("T", &ids(&["A"]), 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].total, 3, "a line counts once however many links");
        let lines: Vec<(usize, &str)> = found[0]
            .contexts
            .iter()
            .map(|c| (c.line, c.snippet.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                (1, "see [[T]] here"),
                (3, "- also [x](T.md#h)"),
                (5, "[[t|again]] and [[T]] twice")
            ]
        );
    }

    #[test]
    fn limits_contexts_but_not_the_total_and_skips_code_and_other_targets() {
        let text = "[[T]]\n[[T]]\n[[T]]\n```\n[[T]]\n```\n`[[T]]`";
        let (_d, v) = vault(&[("T.md", ""), ("A.md", text), ("B.md", "[[Nope]]")]);
        let found = v.link_contexts("T", &ids(&["A", "B", "Gone"]), 2);
        assert_eq!(found.len(), 1, "B and Gone have no such line");
        assert_eq!((found[0].contexts.len(), found[0].total), (2, 3));
    }

    #[test]
    fn long_lines_are_cut_around_the_link() {
        let line = format!("{} [[T]] {}", "word ".repeat(40), "tail ".repeat(40));
        let (_d, v) = vault(&[("T.md", ""), ("A.md", &line)]);
        let found = v.link_contexts("T", &ids(&["A"]), 1);
        let s = &found[0].contexts[0].snippet;
        assert!(
            s.starts_with('…') && s.ends_with('…') && s.contains("[[T]]"),
            "{s}"
        );
        assert!(s.len() < 150, "{}", s.len());
    }

    #[test]
    fn ambiguous_titles_agree_with_clicks() {
        // `[[Old]]` means the shorter id, the root note.
        let (_d, v) = vault(&[
            ("Old.md", ""),
            ("f/Old.md", ""),
            ("A.md", "[[Old]] [[f/Old]]"),
        ]);
        assert_eq!(v.link_contexts("Old", &ids(&["A"]), 5)[0].total, 1);
        assert_eq!(v.link_contexts("f/Old", &ids(&["A"]), 5)[0].total, 1);
    }
}
