#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    pub target: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
}

/// All wikilinks in `content`. A link never spans a line break, so this is
/// [`extract_wikilinks_in_line`] applied per line.
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
fn for_each_wikilink_in_line<'a>(
    line: &'a str,
    mut f: impl FnMut(&'a str, Option<&'a str>, Option<&'a str>),
) {
    let mut i = 0;
    while let Some(start) = line[i..].find("[[") {
        let abs_start = i + start + 2;
        let Some(end) = line[abs_start..].find("]]") else {
            break;
        };
        let abs_end = abs_start + end;
        let inner = &line[abs_start..abs_end];
        i = abs_end + 2;

        if inner.is_empty() {
            continue;
        }

        let (target_part, alias) = match inner.split_once('|') {
            Some((t, a)) => (t, Some(a.trim())),
            None => (inner, None),
        };
        let (target, heading) = match target_part.split_once('#') {
            Some((t, h)) => (t.trim(), Some(h.trim())),
            None => (target_part.trim(), None),
        };

        if !target.is_empty() {
            f(target, heading, alias);
        }
    }
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
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(targets.into_iter().collect());
        }
        for_each_wikilink_in_line(&line, |target, _, _| {
            if !targets.contains(target) {
                targets.insert(target.into());
            }
        });
    }
}

/// The distinct link targets in `text`, sorted (see [`link_targets_from_file`]).
pub fn link_targets(text: &str) -> Box<[Box<str>]> {
    let mut targets: std::collections::BTreeSet<Box<str>> = Default::default();
    for line in text.lines() {
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
}
