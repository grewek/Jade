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
            Some((t, a)) => (t, Some(a.trim().to_string())),
            None => (inner, None),
        };
        let (target, heading) = match target_part.split_once('#') {
            Some((t, h)) => (t.trim().to_string(), Some(h.trim().to_string())),
            None => (target_part.trim().to_string(), None),
        };

        if !target.is_empty() {
            links.push(WikiLink {
                target,
                heading,
                alias,
            });
        }
    }
}

/// The wikilinks in the file at `path`, read line by line so memory stays
/// bounded by the longest line instead of the file size (an 11.8MB note
/// otherwise costs ~11MiB of transient buffer at vault open).
pub fn extract_wikilinks_from_file(path: &std::path::Path) -> std::io::Result<Vec<WikiLink>> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut line = String::new();
    let mut links = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(links);
        }
        extract_wikilinks_in_line(&line, &mut links);
    }
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
    fn file_extraction_matches_in_memory_extraction() {
        let text = "x [[A]] y\r\n[[B#h|al]] [[C]]\nno link\n[[unclosed\n[[D]]";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            extract_wikilinks_from_file(&path).unwrap(),
            extract_wikilinks(text)
        );
    }
}
