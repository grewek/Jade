#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    pub target: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
}

pub fn extract_wikilinks(content: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    let mut i = 0;
    while let Some(start) = content[i..].find("[[") {
        let abs_start = i + start + 2;
        let Some(end) = content[abs_start..].find("]]") else {
            break;
        };
        let abs_end = abs_start + end;
        let inner = &content[abs_start..abs_end];
        i = abs_end + 2;

        if inner.contains('\n') || inner.is_empty() {
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
    links
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
}
