//! Finding the heading a `[[Note#Heading]]` or `[text](note.md#heading)` link
//! points at. Runs once per followed link and walks the note's lines, so it
//! keeps nothing between calls.

use super::document_cache::DocumentCache;
use super::preview::LineRole;

/// The heading text of `line` (`## Title ##` gives `Title`), if it is an ATX
/// heading: one to six `#`, then a space.
fn heading_text(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches('#');
    let hashes = line.len() - rest.len();
    if !(1..=6).contains(&hashes) || !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim_end())
}

/// Words lowercased and joined by single spaces.
fn normalized(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(word.chars().flat_map(char::to_lowercase));
    }
    out
}

/// The first line holding a heading that matches `target`, ignoring case and
/// runs of spaces; a GitHub-style `my-heading` also matches `My Heading`.
/// Fenced code is skipped.
pub(super) fn find_heading_line(text: &str, doc: &DocumentCache, target: &str) -> Option<usize> {
    let wanted = normalized(target);
    if wanted.is_empty() {
        return None;
    }
    let slug_wanted = wanted.replace('-', " ");
    let roles = doc.line_roles();
    (0..doc.line_count()).find(|&line| {
        if roles[line] != LineRole::Plain {
            return false;
        }
        let raw = &text[doc.line_start_byte(line)..doc.line_end_byte(line)];
        heading_text(raw).is_some_and(|heading| {
            let heading = normalized(heading);
            heading == wanted || heading == slug_wanted
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, target: &str) -> Option<usize> {
        find_heading_line(text, &DocumentCache::new(text), target)
    }

    #[test]
    fn matches_ignoring_case_spacing_and_closing_hashes() {
        let text = "# One\n\ntext\n## Two  Words ##\n### three";
        assert_eq!(find(text, "one"), Some(0));
        assert_eq!(find(text, "TWO words"), Some(3));
        assert_eq!(find(text, "Three"), Some(4));
        assert_eq!(find(text, "two-words"), Some(3), "a slug matches too");
        assert_eq!(find(text, "four"), None);
        assert_eq!(find(text, ""), None);
    }

    #[test]
    fn needs_a_space_after_the_hashes_and_at_most_six() {
        assert_eq!(find("#tag\n####### seven\n## ok", "ok"), Some(2));
        assert_eq!(find("#tag", "tag"), None);
        assert_eq!(find("####### seven", "seven"), None);
    }

    #[test]
    fn skips_fenced_code_and_takes_the_first_match() {
        let text = "```\n# Title\n```\n# Title\n# Title";
        assert_eq!(find(text, "title"), Some(3));
    }
}
