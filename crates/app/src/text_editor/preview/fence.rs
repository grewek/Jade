//! Whole-document line classification for fenced code blocks — the one
//! Live Preview construct that can't be told apart one line at a time.
//!
//! Headings, blockquotes, and list markers (`preview/`) don't need
//! this: each is fully determined by its own line's content alone. Whether
//! a line sits *inside* an open code fence, though, depends on whether an
//! earlier line opened one and no later line (before this one) closed it —
//! genuinely whole-document state, however small.
//!
//! Deliberately hand-written (not `markdown::to_mdast`), same style as
//! `preview/`'s own detectors: fence open/close is a simple enough
//! rule (a line starting with 3-or-more of the same fence character, `` ` ``
//! or `~`) to track directly with one sequential pass, rather than paying
//! for a full AST parse on every keystroke just for this.

/// How a `\n`-delimited line should be treated by Live Preview, as far as
/// fenced code blocks are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRole {
    /// Not inside a fence — `preview::build_display_line` applies as
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

/// Whether a fence is open entering a line, and if so, which character and
/// how long its opening run was — the *entire* cross-line state this
/// classifier carries (see [`step`]), which is what makes it resumable from
/// any line given just this one small value.
pub(in crate::text_editor) type FenceState = jade_core::fence::FenceState;

/// Lossless 8-byte form of a [`FenceState`] (24 bytes as an `Option`), for
/// the per-line state array in `document_cache.rs`.
pub(in crate::text_editor) fn pack_fence(state: FenceState) -> u64 {
    match state {
        None => 0,
        Some((ch, len)) => {
            debug_assert!((len as u64) < 1 << 48);
            1 << 56 | u64::from(ch) << 48 | len as u64
        }
    }
}

pub(in crate::text_editor) fn unpack_fence(packed: u64) -> FenceState {
    (packed != 0).then_some((
        (packed >> 48 & 0xFF) as u8,
        (packed & ((1 << 48) - 1)) as usize,
    ))
}

/// The one per-line transition [`classify_lines`] and [`classify_lines_from`]
/// both fold over: given the fence state entering `line`, returns its
/// [`LineRole`] and the fence state *after* it (i.e. entering the next
/// line). Pulled out as its own function so an incremental caller
/// (`document_cache.rs`) can resume classification from an arbitrary line's
/// cached entering state instead of only ever starting fresh from the
/// document's first line.
pub(in crate::text_editor) fn step(state: FenceState, line: &str) -> (LineRole, FenceState) {
    use jade_core::fence::{FenceRole, step};
    let (role, next) = step(state, line);
    let role = match role {
        FenceRole::Plain => LineRole::Plain,
        FenceRole::Delimiter => LineRole::FenceDelimiter,
        FenceRole::Content => LineRole::FenceContent,
    };
    (role, next)
}

/// Classifies every `\n`-delimited line in `source` by [`LineRole`], via a
/// single sequential pass tracking whether a fence is currently open. A
/// fence left unclosed at the end of `source` (e.g. while the user is still
/// typing it) just treats every remaining line as content — there's no
/// later line to notice the fence never closed. A thin fold over [`step`]
/// starting from the empty state — see [`classify_lines_from`] for the
/// incremental counterpart that can start anywhere else.
///
/// `document_cache.rs`'s production path calls `classify_lines_from`
/// directly (it needs the per-line states too, to seed `fence_states`), so
/// this whole-document, roles-only entry point is only still reachable from
/// tests now — kept as the oracle every classify_lines_from-based test
/// (including the incremental path's own) checks itself against, rather
/// than removed once production stopped calling it directly.
#[cfg(test)]
pub(in crate::text_editor) fn classify_lines(source: &str) -> Vec<LineRole> {
    classify_lines_from(source, None)
        .into_iter()
        .map(|(role, _)| role)
        .collect()
}

#[cfg(test)]
/// Resumes classification from `state` (the fence state *entering* the
/// first line of `source`), which is normally a suffix of the whole
/// document starting at some line, not necessarily the document's actual
/// first line. Returns each line's [`LineRole`] paired with the fence state
/// *after* it, so an incremental caller can compare that against a
/// previously cached entering state for the corresponding later line to
/// know the moment it's safe to stop and reuse the old cached suffix
/// unchanged, rather than re-walking the rest of the document — see
/// `document_cache.rs`.
pub(in crate::text_editor) fn classify_lines_from(
    source: &str,
    state: FenceState,
) -> Vec<(LineRole, FenceState)> {
    let mut state = state;
    source
        .split('\n')
        .map(|line| {
            let (role, next) = step(state, line);
            state = next;
            (role, next)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_with_no_fence_is_all_plain() {
        let roles = classify_lines("first\nsecond\nthird");
        assert_eq!(
            roles,
            vec![LineRole::Plain, LineRole::Plain, LineRole::Plain]
        );
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
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceDelimiter
            ]
        );
    }

    #[test]
    fn tilde_fences_are_also_recognized() {
        let source = "~~~\ncode\n~~~";
        let roles = classify_lines(source);
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceDelimiter
            ]
        );
    }

    #[test]
    fn an_unclosed_fence_treats_every_remaining_line_as_content() {
        let source = "```\nstill typing\nthe fence";
        let roles = classify_lines(source);
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceContent
            ]
        );
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
        assert_eq!(
            roles,
            vec![
                LineRole::FenceDelimiter,
                LineRole::FenceContent,
                LineRole::FenceContent
            ]
        );
    }

    #[test]
    fn two_backticks_is_not_a_fence() {
        let roles = classify_lines("``\nnot a fence");
        assert_eq!(roles, vec![LineRole::Plain, LineRole::Plain]);
    }

    /// `classify_lines_from` resuming from an arbitrary line's real entering
    /// state must match `classify_lines`'s own whole-document answer for
    /// that same suffix — the property `document_cache.rs`'s incremental
    /// path actually relies on. Checked for every possible split point
    /// across several documents that exercise open/closed fences, not just
    /// one hand-picked line.
    #[test]
    fn classify_lines_from_resuming_mid_document_matches_a_fresh_whole_document_classification() {
        let documents = [
            "before\n```\nfn main() {}\n```\nafter",
            "```\na\n```\nplain\n```\nb\n```",
            "````\ncode\n```\nstill inside\n````",
            "no fences anywhere\njust\nplain\ntext",
            "~~~\ntilde fence\n~~~\nafter",
        ];
        for source in documents {
            let whole = classify_lines_from(source, None);
            let lines: Vec<&str> = source.split('\n').collect();
            for split in 0..lines.len() {
                let state_entering_split = if split == 0 { None } else { whole[split - 1].1 };
                let suffix = lines[split..].join("\n");
                let resumed = classify_lines_from(&suffix, state_entering_split);
                let expected = &whole[split..];
                assert_eq!(resumed, expected, "source={source:?}, split={split}");
            }
        }
    }
}
