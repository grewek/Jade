//! Obsidian-style callouts: a blockquote whose first line is `[!type]`
//! (optionally with a fold marker `+`/`-` and/or a custom title), e.g.
//!
//! ```text
//! > [!info]
//! > Some additional information
//! ```
//!
//! The *other* block-level construct (besides fenced code/tables) needing
//! cross-line state, for a narrower reason than either of those: telling a
//! callout's *header* line apart from a body line that also happens to
//! start with `>` requires knowing whether the blockquote block just opened
//! on this line or is continuing from above — `live_preview::classify_
//! block_context`'s own `Open::Blockquote` state never needed that
//! distinction (every `>`-prefixed line is already self-sufficient for
//! plain blockquote rendering), so this module tracks it itself via
//! [`classify_callouts`], a sequential pass mirroring `table_preview::
//! classify_tables`'s shape. Deliberately independent of `live_preview.rs`'s
//! own blockquote-prefix detection (same "each scanner is self-contained"
//! precedent `table_preview.rs`/`block_preview.rs` already set) — this
//! module has its own tiny `>`-prefix scan rather than reaching into
//! `live_preview`'s private one.
//!
//! **Top-level only**: a callout header is only ever recognized at
//! blockquote depth 1. A `>>`-nested `[!note]` is never checked — it falls
//! through as plain nested-blockquote content of whatever callout (if any)
//! is already open, same as a callout can't be a list item's content
//! either (untouched by `live_preview::compound_inner`). Consistent with
//! every other one-level compounding cap already in this editor; revisit
//! only if asked to.
//!
//! Colors and icons are deliberately *not* here — [`CalloutType`] is pure
//! data, resolved to an actual `Hsla`/glyph only in `element.rs`, same
//! boundary `table_preview.rs` keeps for its own data.
//!
//! Render-only for now: the fold marker (`+`/`-`) is parsed and hidden (see
//! [`callout_header_prefix`]) so it doesn't leak into the title, but this
//! pass doesn't implement collapsing — every callout always renders fully
//! expanded regardless of which marker (or none) was written. Interactive
//! fold/collapse is explicitly deferred to a later "interactions" pass, not
//! declined — see CLAUDE.md's Backlog.

use std::ops::Range;

/// One of Obsidian's built-in callout type *groups* — several raw type
/// names alias to the same group (e.g. `tip`/`hint`/`important` all share
/// `Tip`'s color/icon), via [`resolve_callout_type`]. An unrecognized
/// custom type name defaults to `Note`, matching real Obsidian behavior
/// (a callout with an unknown type still renders, just with the default
/// styling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutType {
    Note,
    Summary,
    Info,
    Todo,
    Tip,
    Success,
    Question,
    Warning,
    Failure,
    Danger,
    Bug,
    Example,
    Quote,
}

/// Resolves a raw `[!name]` type name (case-insensitive) to the
/// [`CalloutType`] group it belongs to — falls back to `Note` for anything
/// not in Obsidian's own documented alias list.
pub fn resolve_callout_type(name: &str) -> CalloutType {
    match name.to_ascii_lowercase().as_str() {
        "abstract" | "summary" | "tldr" => CalloutType::Summary,
        "info" => CalloutType::Info,
        "todo" => CalloutType::Todo,
        "tip" | "hint" | "important" => CalloutType::Tip,
        "success" | "check" | "done" => CalloutType::Success,
        "question" | "help" | "faq" => CalloutType::Question,
        "warning" | "caution" | "attention" => CalloutType::Warning,
        "failure" | "fail" | "missing" => CalloutType::Failure,
        "danger" | "error" => CalloutType::Danger,
        "bug" => CalloutType::Bug,
        "example" => CalloutType::Example,
        "quote" | "cite" => CalloutType::Quote,
        _ => CalloutType::Note,
    }
}

/// A single Unicode glyph standing in for a callout's icon, per type group
/// — plain data (a `&'static str`, not a GPUI type), unlike color: there's
/// no actual GPUI dependency in choosing *which character* to show, so
/// (unlike `Hsla`) this stays here rather than in `element.rs`.
/// `live_preview::build_callout_header_line` prepends it directly to the
/// header's own rendered text. Chosen from the same general Unicode range
/// the already-shipped bullet/checkbox glyphs (`•`/`☐`/`☑`) come from, for
/// the best chance of actually being in the bundled IBM Plex Mono font —
/// exact glyph choices are a cosmetic detail to eyeball and adjust once
/// seen on screen (can't screenshot-verify, same situation the IBM Plex
/// font rollout itself was in).
pub fn callout_icon(kind: CalloutType) -> &'static str {
    match kind {
        CalloutType::Note => "☆",
        CalloutType::Summary => "☰",
        CalloutType::Info => "ℹ",
        CalloutType::Todo => "☑",
        CalloutType::Tip => "☀",
        CalloutType::Success => "✓",
        CalloutType::Question => "?",
        CalloutType::Warning => "⚠",
        CalloutType::Failure => "✗",
        CalloutType::Danger => "⚡",
        CalloutType::Bug => "☠",
        CalloutType::Example => "☷",
        CalloutType::Quote => "❝",
    }
}

/// A parsed callout header: `[!type]` plus an optional fold marker and an
/// optional custom title, resolved from `content` — the text right after
/// the blockquote's own `>` prefix has already been stripped by the
/// caller.
pub struct CalloutHeader {
    pub kind: CalloutType,
    /// Byte range of the raw type name as typed (e.g. `important`, not
    /// `Tip`) — shown capitalized in place of a custom title when there
    /// isn't one, matching Obsidian's own behavior of displaying what was
    /// actually typed rather than the resolved alias group's own name.
    pub type_name: Range<usize>,
    /// Byte offset, relative to `content`, right after the `[!type]+`/`-`
    /// marker itself (not including any separating space) — callers hide
    /// `content[..marker_end]` unconditionally.
    pub marker_end: usize,
    /// Byte range of the custom title text within `content`, if any (a
    /// single separating space right after the marker is trimmed off, if
    /// present). `None` when nothing follows the marker at all — the
    /// caller falls back to the capitalized type name instead.
    pub title: Option<Range<usize>>,
}

/// Parses `[!type]` (+ optional `+`/`-` fold marker, + optional ` title`)
/// at the very start of `content`, if present. The type name may be any
/// non-empty run of characters up to the closing `]` — [`resolve_callout_type`]
/// is what maps it to a known group (or `Note`, if unrecognized); this
/// function doesn't reject an unknown name, matching Obsidian's own
/// leniency.
pub fn callout_header_prefix(content: &str) -> Option<CalloutHeader> {
    let bytes = content.as_bytes();
    if bytes.first() != Some(&b'[') || bytes.get(1) != Some(&b'!') {
        return None;
    }
    let close = content.find(']')?;
    let name = &content[2..close];
    if name.is_empty() {
        return None;
    }
    let kind = resolve_callout_type(name);
    let type_name = 2..close;

    let mut marker_end = close + 1;
    if bytes
        .get(marker_end)
        .is_some_and(|&b| b == b'+' || b == b'-')
    {
        marker_end += 1;
    }

    let title = if marker_end >= content.len() {
        None
    } else {
        let title_start = if bytes.get(marker_end) == Some(&b' ') {
            marker_end + 1
        } else {
            marker_end
        };
        (title_start < content.len()).then_some(title_start..content.len())
    };

    Some(CalloutHeader {
        kind,
        type_name,
        marker_end,
        title,
    })
}

/// One detected callout: its type, and the line range (`end_line`
/// exclusive) it and its body span — enough for `element.rs` to paint a
/// background/accent bar across the whole block and pick a color/icon.
/// Deliberately doesn't carry the parsed header itself (title, fold marker)
/// — unlike a table's column widths, a header is resolvable from its own
/// single line alone, so `live_preview::build_callout_header_line`
/// re-parses it fresh via [`callout_header_prefix`] rather than this
/// struct caching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callout {
    pub kind: CalloutType,
    pub start_line: usize,
    pub end_line: usize,
}

impl Callout {
    /// Whether `line` is this callout's header — the one line with `[!type]`
    /// on it, showing the title/icon instead of ordinary body content.
    pub fn is_header(&self, line: usize) -> bool {
        line == self.start_line
    }
}

/// A blockquote `>` prefix's nesting depth and raw byte length — this
/// module's own tiny copy of `live_preview::blockquote_prefix`'s scan (see
/// this module's own doc comment for why it doesn't just reuse that
/// private function).
fn blockquote_depth(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut depth: u8 = 0;
    while bytes.get(i) == Some(&b'>') {
        i += 1;
        depth += 1;
        if bytes.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    if depth == 0 { None } else { Some((depth, i)) }
}

/// Whether `line` looks like an ATX heading (`#` through `######`, followed
/// by a space) — used only to decide whether it closes an open blockquote/
/// callout block, same CommonMark rule `live_preview::classify_block_
/// context` already applies for the same reason.
fn looks_like_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ')
}

/// Resolves, for every line of `source`, which [`Callout`] (if any) it
/// belongs to — a sequential pass mirroring `table_preview::
/// classify_tables`'s shape. Tracks two bits of state: `block_open` (was
/// the *previous* line part of an open blockquote block at all, callout or
/// not — this is what lets a line tell "I'm a fresh blockquote start" apart
/// from "I'm continuing one," the distinction a callout header check needs
/// and plain blockquote rendering never did) and `current` (the currently-
/// open callout's index into the result, if the open block happens to be
/// one).
///
/// A line opens a new `Callout` when it's a *fresh*, depth-1 blockquote
/// line (see [`blockquote_depth`]) whose content matches
/// [`callout_header_prefix`]. It extends through every following line
/// still part of that same blockquote block — a further `>`-prefixed line
/// at any depth (a `>>`-nested line stays part of the body, rendered as a
/// plain nested blockquote, not checked as a header of its own — see this
/// module's own top-level-only doc note) or a lazy-continuation line with
/// no prefix at all — until a blank line, a heading, or a fresh
/// non-continuing blockquote line closes the block, mirroring
/// `live_preview::classify_block_context`'s own closing rules for
/// `Open::Blockquote`.
///
/// `document_cache.rs`'s production path calls `classify_callouts_from`
/// directly (it needs the per-line states and running callout count too,
/// to seed its own cache) — this whole-document entry point is only still
/// reachable from tests now, kept as the oracle every classify_callouts_
/// from-based test (including the incremental path's own) checks itself
/// against, same role `block_preview::classify_lines` now plays there.
#[cfg(test)]
pub(super) fn classify_callouts(source: &str) -> (Vec<Option<usize>>, Vec<Callout>) {
    let mut callouts = Vec::new();
    let results = classify_callouts_from(source, CalloutState::default(), 0, &mut callouts);
    let callout_for_line = results.into_iter().map(|(callout, _, _)| callout).collect();
    (callout_for_line, callouts)
}

/// The cross-line state [`step`] carries: `block_open` (was the *previous*
/// line part of an open blockquote block at all) and `current` (the index
/// into the caller's `callouts` of the currently-open callout, if the open
/// block happens to be one). The *entire* state this classifier needs,
/// which — together with `callouts.len()` itself, since a resumed walk's
/// freshly assigned indices need to agree with what the original run would
/// have assigned at the same point — is what makes it resumable; see
/// `document_cache.rs`'s own `patch_callouts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CalloutState {
    block_open: bool,
    pub(super) current: Option<usize>,
}

/// The one per-line transition [`classify_callouts`]/[`classify_callouts_from`]
/// both fold over — pulled out of the original single sequential loop the
/// same way `block_preview::step` was, so an incremental caller can resume
/// from an arbitrary line's cached entering state instead of only ever
/// starting fresh from line 0. `i` is the line's *absolute* index (needed
/// for `Callout::start_line`/`end_line`, unlike `block_preview::step`,
/// which never needed to know where it was). `callouts` is mutated in
/// place — a freshly opened callout is pushed with index `callouts.len()`,
/// and a continuing one has its `end_line` bumped — so a resumed caller
/// that's already pre-seeded `callouts` with a kept prefix assigns new
/// indices continuing on from that prefix, not colliding with it.
pub(super) fn step(
    state: CalloutState,
    line: &str,
    i: usize,
    callouts: &mut Vec<Callout>,
) -> (Option<usize>, CalloutState) {
    let CalloutState {
        block_open,
        mut current,
    } = state;

    if line.trim().is_empty() || looks_like_heading(line) {
        if let Some(idx) = current.take() {
            callouts[idx].end_line = i;
        }
        return (
            None,
            CalloutState {
                block_open: false,
                current,
            },
        );
    }

    if let Some((depth, prefix_len)) = blockquote_depth(line) {
        let mut callout_for_this_line = None;
        if depth == 1 && !block_open {
            // A fresh depth-1 start: check whether it opens a callout.
            current = None;
            if let Some(header) = callout_header_prefix(&line[prefix_len..]) {
                let idx = callouts.len();
                callouts.push(Callout {
                    kind: header.kind,
                    start_line: i,
                    end_line: i + 1,
                });
                callout_for_this_line = Some(idx);
                current = Some(idx);
            }
        } else if let Some(idx) = current {
            // Continuing an already-open callout's body, at any depth.
            callouts[idx].end_line = i + 1;
            callout_for_this_line = Some(idx);
        }
        return (
            callout_for_this_line,
            CalloutState {
                block_open: true,
                current,
            },
        );
    }

    // No `>` prefix of its own: a lazy continuation if a block is open,
    // otherwise an ordinary line that closes whatever was open.
    let callout_for_this_line = if block_open {
        if let Some(idx) = current {
            callouts[idx].end_line = i + 1;
            Some(idx)
        } else {
            None
        }
    } else {
        if let Some(idx) = current.take() {
            callouts[idx].end_line = i;
        }
        current = None;
        None
    };
    (
        callout_for_this_line,
        CalloutState {
            block_open,
            current,
        },
    )
}

/// Resumes classification from `state` (the state *entering* the first
/// line of `source`, whose absolute document line index is `start_line`)
/// — `source` is normally a suffix of the whole document, not necessarily
/// its actual start. `callouts` should already hold whatever prefix of
/// callouts a resumed caller is keeping unchanged — new ones are appended
/// starting at `callouts.len()`. Returns each line's own callout index
/// (if any), the state *after* it, and `callouts.len()` *after* it — the
/// latter is what an incremental caller needs on top of the state itself
/// to know a resumed line's freshly assigned index actually lines up with
/// a previously cached one (see `document_cache.rs`'s own `patch_callouts`
/// doc comment for why state equality alone isn't enough here, unlike
/// `block_preview`'s simpler case).
pub(super) fn classify_callouts_from(
    source: &str,
    state: CalloutState,
    start_line: usize,
    callouts: &mut Vec<Callout>,
) -> Vec<(Option<usize>, CalloutState, usize)> {
    let mut state = state;
    source
        .split('\n')
        .enumerate()
        .map(|(k, line)| {
            let (callout, next_state) = step(state, line, start_line + k, callouts);
            state = next_state;
            (callout, next_state, callouts.len())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_callout_type_matches_known_aliases() {
        assert_eq!(resolve_callout_type("info"), CalloutType::Info);
        assert_eq!(resolve_callout_type("INFO"), CalloutType::Info);
        assert_eq!(resolve_callout_type("hint"), CalloutType::Tip);
        assert_eq!(resolve_callout_type("important"), CalloutType::Tip);
        assert_eq!(resolve_callout_type("done"), CalloutType::Success);
        assert_eq!(resolve_callout_type("caution"), CalloutType::Warning);
        assert_eq!(resolve_callout_type("error"), CalloutType::Danger);
        assert_eq!(resolve_callout_type("cite"), CalloutType::Quote);
    }

    #[test]
    fn resolve_callout_type_falls_back_to_note_for_unknown_names() {
        assert_eq!(resolve_callout_type("made-up-type"), CalloutType::Note);
    }

    #[test]
    fn callout_header_prefix_parses_the_bare_type() {
        let header = callout_header_prefix("[!info]").unwrap();
        assert_eq!(header.kind, CalloutType::Info);
        assert_eq!(header.marker_end, 7);
        assert!(header.title.is_none());
    }

    #[test]
    fn callout_header_prefix_parses_a_custom_title() {
        let content = "[!info] Custom Title";
        let header = callout_header_prefix(content).unwrap();
        assert_eq!(&content[header.title.unwrap()], "Custom Title");
    }

    #[test]
    fn callout_header_prefix_hides_the_fold_marker_either_way() {
        for content in ["[!info]+", "[!info]-", "[!info]+ Title", "[!info]- Title"] {
            let header = callout_header_prefix(content).unwrap();
            assert!(!content[..header.marker_end].is_empty());
            assert!(content[..header.marker_end].ends_with(['+', '-']));
        }
    }

    #[test]
    fn callout_header_prefix_rejects_non_callout_brackets() {
        assert!(callout_header_prefix("[not a callout]").is_none());
        assert!(callout_header_prefix("plain text").is_none());
        assert!(callout_header_prefix("[!]").is_none());
    }

    #[test]
    fn classify_callouts_recognizes_a_simple_callout() {
        let source = "> [!info]\n> Some additional information";
        let (callout_for_line, callouts) = classify_callouts(source);
        assert_eq!(callout_for_line, vec![Some(0), Some(0)]);
        assert_eq!(callouts.len(), 1);
        assert_eq!(callouts[0].kind, CalloutType::Info);
        assert_eq!(callouts[0].start_line, 0);
        assert_eq!(callouts[0].end_line, 2);
        assert!(callouts[0].is_header(0));
        assert!(!callouts[0].is_header(1));
    }

    #[test]
    fn classify_callouts_ends_at_a_blank_line() {
        let source = "> [!warning]\n> body\n\n> [!note]\n> other body";
        let (_, callouts) = classify_callouts(source);
        assert_eq!(callouts.len(), 2);
        assert_eq!(callouts[0].end_line, 2);
        assert_eq!(callouts[1].start_line, 3);
    }

    #[test]
    fn classify_callouts_ends_at_a_heading() {
        let source = "> [!note]\n> body\n# Heading";
        let (callout_for_line, callouts) = classify_callouts(source);
        assert_eq!(callouts[0].end_line, 2);
        assert_eq!(callout_for_line[2], None);
    }

    #[test]
    fn classify_callouts_includes_a_lazy_continuation_line() {
        let source = "> [!tip]\n> first\nsecond with no marker";
        let (callout_for_line, callouts) = classify_callouts(source);
        assert_eq!(callout_for_line, vec![Some(0), Some(0), Some(0)]);
        assert_eq!(callouts[0].end_line, 3);
    }

    #[test]
    fn a_plain_blockquote_is_not_a_callout() {
        let source = "> just a regular quote\n> second line";
        let (callout_for_line, callouts) = classify_callouts(source);
        assert_eq!(callout_for_line, vec![None, None]);
        assert!(callouts.is_empty());
    }

    #[test]
    fn a_bracketed_line_mid_block_is_not_treated_as_a_header() {
        // `[!info]`-looking text on a line that isn't the *fresh* start of
        // a blockquote block must not retroactively become a callout.
        let source = "> some text\n> [!info] not a header, just content";
        let (callout_for_line, callouts) = classify_callouts(source);
        assert_eq!(callout_for_line, vec![None, None]);
        assert!(callouts.is_empty());
    }

    #[test]
    fn a_nested_callout_header_is_not_recognized_top_level_only() {
        let source = "> [!info]\n> outer body\n>> [!note]\n>> nested body";
        let (callout_for_line, callouts) = classify_callouts(source);
        // The whole block (including the nested lines) still belongs to
        // the outer callout — the nested `[!note]` is just plain nested
        // blockquote content, never its own callout.
        assert_eq!(callout_for_line, vec![Some(0), Some(0), Some(0), Some(0)]);
        assert_eq!(callouts.len(), 1);
        assert_eq!(callouts[0].kind, CalloutType::Info);
    }

    #[test]
    fn an_unrecognized_type_still_opens_a_callout_as_note() {
        let source = "> [!custom-type]\n> body";
        let (_, callouts) = classify_callouts(source);
        assert_eq!(callouts.len(), 1);
        assert_eq!(callouts[0].kind, CalloutType::Note);
    }
}
