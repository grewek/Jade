use crate::fence::{FenceRole, FenceWalk};
use crate::link_scan::{LinkKind, scan_line};
use crate::math_scan::{BLOCK_LOOKAHEAD, block_len, may_open_block, prose_lines};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    pub target: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
}

/// All wikilinks in `content`. A link never spans a line break, so this is
/// `extract_wikilinks_in_line` applied per line.
pub fn extract_wikilinks(content: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    for line in content.lines() {
        extract_wikilinks_in_line(line, &mut links);
    }
    links
}

/// The wikilinks in one `line` (no line break inside), appended to `links`.
pub fn extract_wikilinks_in_line(line: &str, links: &mut Vec<WikiLink>) {
    for_each_wikilink_in_line(line, |target, heading, alias| {
        links.push(WikiLink {
            target: target.to_string(),
            heading: heading.map(str::to_string),
            alias: alias.map(str::to_string),
        });
    });
}

/// Calls `f(target, heading, alias)` for each wikilink in `line`, borrowing
/// from it so callers that need less than a whole [`WikiLink`] allocate less.
/// The grammar is [`crate::link_scan`]'s, so a link inside an inline code
/// span or after an escape is not counted, as it is not rendered as one.
fn for_each_wikilink_in_line<'a>(
    line: &'a str,
    mut f: impl FnMut(&'a str, Option<&'a str>, Option<&'a str>),
) {
    // Most lines have none, and this runs for every line of every note at
    // vault open.
    if !line.contains("[[") {
        return;
    }
    scan_line(line, |link| {
        if link.kind == LinkKind::Wiki {
            f(
                &line[link.dest],
                link.heading.map(|h| &line[h]),
                link.alias.map(|a| line[a].trim()),
            );
        }
    });
}

/// The distinct link targets in the file at `path`, sorted. This is all the
/// vault's backlink index needs (not headings or aliases), and notes
/// typically repeat a few targets many times, so it stores far less than
/// every [`WikiLink`]. Read line by line, so memory is bounded by the longest
/// line, not the file (an 11.8MB note otherwise costs a ~14MiB transient peak
/// at vault open).
pub fn link_targets_from_file(path: &std::path::Path) -> std::io::Result<Box<[Box<str>]>> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut line = String::new();
    let mut targets: std::collections::BTreeSet<Box<str>> = Default::default();
    let mut fences = FenceWalk::default();
    // This loop runs on every line of a huge note at vault open, so it carries
    // no lookahead: it stops at the first line that may open a `$$` block and
    // the rest of the file goes through the slower loop (most notes have no math).
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(targets.into_iter().collect());
        }
        // The editor's lines exclude the newline, and fence rules see that.
        let text = line.strip_suffix('\n').unwrap_or(&line);
        if may_open_block(text) {
            break;
        }
        if fences.role(text) == FenceRole::Plain {
            for_each_wikilink_in_line(text, |target, _, _| {
                if !targets.contains(target) {
                    targets.insert(target.into());
                }
            });
        }
    }
    let mut lines = Lines::new(reader);
    lines.ahead.push_back(line);
    targets_with_math(&mut lines, &mut targets, &mut fences)?;
    Ok(targets.into_iter().collect())
}

fn add_targets(text: &str, targets: &mut std::collections::BTreeSet<Box<str>>) {
    for_each_wikilink_in_line(text, |target, _, _| {
        if !targets.contains(target) {
            targets.insert(target.into());
        }
    });
}

/// The rest of the input, skipping `$$` blocks.
fn targets_with_math(
    lines: &mut Lines<impl std::io::BufRead>,
    targets: &mut std::collections::BTreeSet<Box<str>>,
    fences: &mut FenceWalk,
) -> std::io::Result<()> {
    let mut buf = String::new();
    while lines.next_into(&mut buf)? {
        let text = buf.strip_suffix('\n').unwrap_or(&buf);
        if fences.role(text) != FenceRole::Plain {
            continue;
        }
        if may_open_block(text) && lines.skip_block(text)? {
            continue;
        }
        add_targets(text, targets);
    }
    Ok(())
}

/// A file's lines, newline kept, with room to look ahead.
struct Lines<R> {
    reader: R,
    /// Lines read ahead to decide whether a `$$` opens a block, not yet handled.
    ahead: std::collections::VecDeque<String>,
}

impl<R: std::io::BufRead> Lines<R> {
    fn new(reader: R) -> Self {
        Lines {
            reader,
            ahead: Default::default(),
        }
    }

    /// Replaces `buf` with the next line; `false` at the end of the input.
    fn next_into(&mut self, buf: &mut String) -> std::io::Result<bool> {
        if let Some(line) = self.ahead.pop_front() {
            *buf = line;
            return Ok(true);
        }
        buf.clear();
        self.read_into(buf)
    }

    fn read_into(&mut self, line: &mut String) -> std::io::Result<bool> {
        Ok(self.reader.read_line(line)? != 0)
    }

    /// Whether the `$$` line `opener` opens a block, in which case the rest of
    /// its lines are consumed (the opener already is). Out of line: the common
    /// line never gets here.
    #[inline(never)]
    fn skip_block(&mut self, opener: &str) -> std::io::Result<bool> {
        while self.ahead.len() < BLOCK_LOOKAHEAD {
            let mut line = String::new();
            if !self.read_into(&mut line)? {
                break;
            }
            self.ahead.push_back(line);
        }
        let held: Vec<&str> = std::iter::once(opener)
            .chain(self.ahead.iter().map(|l| l.strip_suffix('\n').unwrap_or(l)))
            .collect();
        let Some(len) = block_len(&held) else {
            return Ok(false);
        };
        // The block's lines are math: the fence walk never sees them.
        self.ahead.drain(..len - 1);
        Ok(true)
    }
}

/// The distinct link targets in `text`, sorted (see [`link_targets_from_file`]).
pub fn link_targets(text: &str) -> Box<[Box<str>]> {
    let mut targets: std::collections::BTreeSet<Box<str>> = Default::default();
    for (_, line) in prose_lines(text) {
        for_each_wikilink_in_line(line, |target, _, _| {
            if !targets.contains(target) {
                targets.insert(target.into());
            }
        });
    }
    targets.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_in_fenced_code_are_not_targets_from_text_or_file() {
        let text = "[[a]]\n```\n[[b]]\n```\n[[c]]\n~~~\n[[d]]";
        let want: Vec<&str> = vec!["a", "c"];
        let got = |t: &[Box<str>]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(got(&link_targets(text)), want);
        let dir = std::env::temp_dir().join(format!("jade-fence-{}", std::process::id()));
        std::fs::write(&dir, text).unwrap();
        let from_file = link_targets_from_file(&dir).unwrap();
        std::fs::remove_file(&dir).unwrap();
        assert_eq!(got(&from_file), want);
    }

    #[test]
    fn plain_link() {
        let links = extract_wikilinks("See [[Other Note]] for details.");
        assert_eq!(
            links,
            vec![WikiLink {
                target: "Other Note".into(),
                heading: None,
                alias: None,
            }]
        );
    }

    #[test]
    fn aliased_link() {
        let links = extract_wikilinks("[[Other Note|display text]]");
        assert_eq!(
            links,
            vec![WikiLink {
                target: "Other Note".into(),
                heading: None,
                alias: Some("display text".into()),
            }]
        );
    }

    #[test]
    fn link_with_heading_and_alias() {
        let links = extract_wikilinks("[[Other Note#Section|see here]]");
        assert_eq!(
            links,
            vec![WikiLink {
                target: "Other Note".into(),
                heading: Some("Section".into()),
                alias: Some("see here".into()),
            }]
        );
    }

    #[test]
    fn multiple_links() {
        let links = extract_wikilinks("[[A]] and [[B]]");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, "A");
        assert_eq!(links[1].target, "B");
    }

    #[test]
    fn unclosed_link_is_ignored() {
        let links = extract_wikilinks("This has [[ no closing brackets");
        assert!(links.is_empty());
    }

    #[test]
    fn a_link_never_spans_lines_and_does_not_hide_the_next_one() {
        let links = extract_wikilinks("[[a\n[[b]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "b");
        assert!(extract_wikilinks("[[a\nb]]").is_empty());
    }

    #[test]
    fn link_targets_are_distinct_sorted_and_match_the_full_extraction() {
        let text = "[[b#h|x]] [[a]] [[b]]\n[[a|y]]\n[[ c ]]";
        let expected: Vec<String> = {
            let mut t: Vec<String> = extract_wikilinks(text)
                .into_iter()
                .map(|l| l.target)
                .collect();
            t.sort();
            t.dedup();
            t
        };
        let got: Vec<String> = link_targets(text).iter().map(|t| t.to_string()).collect();
        assert_eq!(got, expected);
        assert_eq!(got, ["a", "b", "c"]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, text).unwrap();
        assert_eq!(link_targets_from_file(&path).unwrap(), link_targets(text));
    }

    #[test]
    fn links_in_code_spans_and_after_escapes_are_not_wikilinks() {
        let links = extract_wikilinks("`[[a]]` \\[[b]] [[c]] **[[d]]**");
        let targets: Vec<_> = links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["c", "d"]);
    }

    #[test]
    fn links_in_a_math_block_are_not_targets_from_text_or_file() {
        let got = |t: &[Box<str>]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let dir = std::env::temp_dir();
        let cases = [
            ("[[a]]\n$$\n[[b]]\nx\n$$\n[[c]]", vec!["a", "c"]),
            // No closer before the blank line: not a block, so [[b]] counts.
            ("[[a]]\n$$\n[[b]]\n\n[[c]]", vec!["a", "b", "c"]),
            ("$$\nx\n$$ [[same line]]\n[[after]]", vec!["after"]),
            ("```\n$$\n```\n[[a]]\n$$\n[[b]]", vec!["a", "b"]),
        ];
        for (n, (text, want)) in cases.into_iter().enumerate() {
            let path = dir.join(format!("jade-math-{}-{n}", std::process::id()));
            std::fs::write(&path, text).unwrap();
            assert_eq!(got(&link_targets(text)), want, "text {text:?}");
            assert_eq!(
                got(&link_targets_from_file(&path).unwrap()),
                want,
                "file {text:?}"
            );
        }
    }

    #[test]
    fn the_streamed_and_whole_text_targets_agree_on_random_notes() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut below = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let lines = [
            "$$",
            "$$ x",
            "x $$",
            "[[a]]",
            "[[b]]",
            "",
            "```",
            "text",
            "$$ y $$ [[c]]",
            "  $$",
        ];
        let path = std::env::temp_dir().join(format!("jade-math-diff-{}", std::process::id()));
        for _ in 0..400 {
            let n = 1 + below(14);
            let text: String = (0..n)
                .map(|_| lines[below(lines.len())])
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&path, &text).unwrap();
            assert_eq!(
                link_targets(&text),
                link_targets_from_file(&path).unwrap(),
                "{text:?}"
            );
        }
    }
}
