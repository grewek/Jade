//! Whole-document line classification for fenced code blocks — the one
//! Live Preview construct that can't be told apart one line at a time.
//!
//! Headings, blockquotes, and list markers (`live_preview.rs`) don't need
//! this: each is fully determined by its own line's content alone. Whether
//! a line sits *inside* an open code fence, though, depends on whether an
//! earlier line opened one and no later line (before this one) closed it —
//! genuinely whole-document state, however small.
//!
//! Deliberately hand-written (not `markdown::to_mdast`), same style as
//! `live_preview.rs`'s own detectors: fence open/close is a simple enough
//! rule (a line starting with 3-or-more of the same fence character, `` ` ``
//! or `~`) to track directly with one sequential pass, rather than paying
//! for a full AST parse on every keystroke just for this.

/// How a `\n`-delimited line should be treated by Live Preview, as far as
/// fenced code blocks are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRole {
    /// Not inside a fence — `live_preview::build_display_line` applies as
    /// normal (headings/emphasis/code-spans/wikilinks/blockquotes/lists).
    Plain,
    /// The opening or closing fence line itself (e.g. `` ```rust `` or
    /// `` ``` ``) — hidden when inactive, shown raw when active, same as
    /// any other markup delimiter.
    FenceDelimiter,
    /// A line strictly between an opening and closing fence — always shown
    /// literal, never inline-scanned, whether or not it's the active line:
    /// code fence content is never Markdown, so there's nothing to preview.
    FenceContent,
}

/// Classifies every `\n`-delimited line in `source` by [`LineRole`], via a
/// single sequential pass tracking whether a fence is currently open. A
/// fence left unclosed at the end of `source` (e.g. while the user is still
/// typing it) just treats every remaining line as content — there's no
/// later line to notice the fence never closed.
pub fn classify_lines(source: &str) -> Vec<LineRole> {
    let mut roles = Vec::with_capacity(source.split('\n').count());
    let mut open_fence: Option<(u8, usize)> = None;

    for line in source.split('\n') {
        match open_fence {
            None => {
                if let Some(fence) = fence_delimiter(line) {
                    open_fence = Some(fence);
                    roles.push(LineRole::FenceDelimiter);
                } else {
                    roles.push(LineRole::Plain);
                }
            }
            Some((fence_char, min_len)) => {
                if closes_fence(line, fence_char, min_len) {
                    open_fence = None;
                    roles.push(LineRole::FenceDelimiter);
                } else {
                    roles.push(LineRole::FenceContent);
                }
            }
        }
    }
    roles
}

/// `line`'s fence character and run length, if it opens a code fence: 3 or
/// more `` ` `` or `~` at the very start of the line (CommonMark's
/// allowance for up to 3 leading spaces isn't handled — same simplification
/// `live_preview.rs`'s `heading_prefix` makes), optionally followed by a
/// language info string (its content doesn't matter for classification,
/// just that the fence-character run itself is long enough).
fn fence_delimiter(line: &str) -> Option<(u8, usize)> {
    let fence_char = *line.as_bytes().first()?;
    if fence_char != b'`' && fence_char != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == fence_char).count();
    if len < 3 {
        return None;
    }
    Some((fence_char, len))
}

/// Whether `line` closes a fence opened with `fence_char` repeated at least
/// `min_len` times. CommonMark allows trailing whitespace after the closing
/// run and requires only that it be at least as long as the opener;
/// simplified here to "a run of exactly `fence_char`, at least `min_len`
/// long, and nothing else on the line".
fn closes_fence(line: &str, fence_char: u8, min_len: usize) -> bool {
    let len = line.bytes().take_while(|&b| b == fence_char).count();
    len >= min_len && len == line.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_with_no_fence_is_all_plain() {
        let roles = classify_lines("first\nsecond\nthird");
        assert_eq!(roles, vec![LineRole::Plain, LineRole::Plain, LineRole::Plain]);
    }

    #[test]
    fn a_simple_fence_classifies_delimiters_and_content_separately() {
        let source = "before\n```\nfn main() {}\n```\nafter";
        let roles = classify_lines(source);
        assert_eq!(
            roles,
            vec![
                LineRole::Plain,
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceDelimiter,
                LineRole::Plain,
            ]
        );
    }

    #[test]
    fn a_fence_with_a_language_info_string_is_still_a_delimiter() {
        let source = "```rust\ncode\n```";
        let roles = classify_lines(source);
        assert_eq!(roles, vec![LineRole::FenceDelimiter, LineRole::FenceContent, LineRole::FenceDelimiter]);
    }

    #[test]
    fn tilde_fences_are_also_recognized() {
        let source = "~~~\ncode\n~~~";
        let roles = classify_lines(source);
        assert_eq!(roles, vec![LineRole::FenceDelimiter, LineRole::FenceContent, LineRole::FenceDelimiter]);
    }

    #[test]
    fn an_unclosed_fence_treats_every_remaining_line_as_content() {
        let source = "```\nstill typing\nthe fence";
        let roles = classify_lines(source);
        assert_eq!(roles, vec![LineRole::FenceDelimiter, LineRole::FenceContent, LineRole::FenceContent]);
    }

    #[test]
    fn multiple_fences_in_one_document_dont_bleed_into_each_other() {
        let source = "```\na\n```\nplain\n```\nb\n```";
        let roles = classify_lines(source);
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceDelimiter,
                LineRole::Plain,
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceDelimiter,
            ]
        );
    }

    #[test]
    fn a_closing_fence_shorter_than_the_opener_does_not_close_it() {
        // CommonMark requires the closing run to be at least as long as the
        // opening one.
        let source = "````\ncode\n```\nstill inside\n````";
        let roles = classify_lines(source);
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceContent, // too-short "```" doesn't close a "````" fence
                LineRole::FenceContent,
                LineRole::FenceDelimiter,
            ]
        );
    }

    #[test]
    fn trailing_characters_after_the_closing_run_prevent_it_from_closing() {
        let source = "```\ncode\n```extra";
        let roles = classify_lines(source);
        assert_eq!(roles, vec![LineRole::FenceDelimiter, LineRole::FenceContent, LineRole::FenceContent]);
    }

    #[test]
    fn two_backticks_is_not_a_fence() {
        let roles = classify_lines("``\nnot a fence");
        assert_eq!(roles, vec![LineRole::Plain, LineRole::Plain]);
    }
}
