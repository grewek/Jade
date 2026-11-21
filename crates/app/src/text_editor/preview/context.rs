//! Cross-line block context: list nesting depth and lazy continuation.

use super::*;

/// `line`'s structural indent (see [`LineIndent`]), from the same
/// `blockquote_prefix`/`list_marker` detection `build_display_line` uses for
/// hiding/replacing markup — computed separately (rather than threaded out
/// of those branches) so it's available even on the active line, which skips
/// them entirely.
pub(in crate::text_editor) fn line_indent(line: &str) -> LineIndent {
    if let Some((depth, _)) = blockquote_prefix(line) {
        return LineIndent::Blockquote(depth);
    }
    if indented_list_marker(line).is_some() {
        return LineIndent::List(1);
    }
    LineIndent::None
}

/// What [`classify_block_context`] resolved a line to need, beyond what
/// [`build_display_line`] can tell from that line alone — consumed by
/// [`build_display_line_in_context`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineContext {
    /// Nothing from whole-document context is needed — `build_display_line`
    /// already resolves this line correctly by itself (not part of any
    /// list/blockquote, or it has its own unambiguous `>`/marker prefix).
    Plain,
    /// An indented list marker line, at `depth` (1 = top-level) resolved
    /// from its indentation relative to whichever ancestor items were still
    /// open — `build_display_line` alone would assume depth `1` regardless
    /// of indentation (see its own doc comment).
    ListMarkerLine(u8),
    /// A line with no blockquote/list prefix of its own, still treated as
    /// continuing the enclosing block's content (CommonMark "lazy
    /// continuation") at this [`LineIndent`].
    LazyContinuation(LineIndent),
}

/// Resolves, for every line of the whole `source` document, the two things
/// [`build_display_line`] can't tell from a single line in isolation:
/// nested-list depth and lazy continuation — see this module's own doc
/// comment for why both need cross-line state, unlike everything else this
/// module detects. A single sequential pass (mirroring
/// `preview::fence::classify_lines`'s fence-tracking state machine), tracking
/// whichever block is currently "open":
///
/// - A blockquote `>` line opens [`LineIndent::Blockquote`] at its depth.
/// - An (optionally indented) list marker line opens or continues a stack of
///   nesting levels, keyed by each level's own indentation column: deeper
///   levels (indented further than the current marker) are popped, a level
///   at the *same* column is reused (a sibling item, not a new level), and
///   anything strictly shallower stays open as this marker's ancestor.
/// - A blank line or a heading (CommonMark: both always interrupt an open
///   paragraph) closes whatever was open.
/// - Any other non-blank line with no marker of its own is a lazy
///   continuation of whatever's still open, *regardless of its own
///   indentation* — deliberately as lenient as a blockquote's lazy
///   continuation already is, not CommonMark's stricter per-list-item
///   content-column alignment rule, since this module doesn't track each
///   level's content-start column, only where its marker began.
///
/// `document_cache.rs`'s production path calls `classify_block_context_from`
/// directly (it needs the per-line states too, to seed its own cache) — this
/// whole-document entry point is only still reachable from tests now, kept
/// as the oracle every classify_block_context_from-based test (including
/// the incremental path's own) checks itself against, same role
/// `preview::fence::classify_lines`/`callout::classify_callouts` now play
/// there.
#[cfg(test)]
pub(in crate::text_editor) fn classify_block_context(source: &str) -> Vec<LineContext> {
    classify_block_context_from(source, OpenBlock::None)
        .into_iter()
        .map(|(ctx, _)| ctx)
        .collect()
}

/// The cross-line state [`block_context_step`] carries — see
/// [`classify_block_context`]'s own doc comment for what each variant
/// means. Pulled out to module scope (rather than kept as a function-local
/// `enum`, which is what it originally was) so an incremental caller
/// (`document_cache.rs`) can store and compare it across paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::text_editor) enum OpenBlock {
    None,
    Blockquote(u8),
    /// The currently-open list's nesting stack, top = innermost level —
    /// see [`ListNode`]'s own doc comment for why this is a persistent
    /// `Rc`-linked chain rather than a `Vec<usize>`. Never `None` while
    /// this variant is current — see the marker-line branch below, which
    /// always leaves at least one level.
    List(Rc<ListNode>),
}

/// One level of [`OpenBlock::List`]'s nesting stack, linked to its parent
/// (the next level out) via `Rc` rather than stored inline in a `Vec` —
/// replaces what used to be a plain `Vec<usize>` that `classify_block_
/// context_from`'s loop deep-cloned (`open = next.clone()`) on *every*
/// line inside an open list, not just where nesting depth actually
/// changed (confirmed by direct measurement: ~18MB / 98,642 separate
/// heap allocations on the 320K-line stress fixture, where 21.9% of all
/// lines are inside some open list — see HISTORY.md).
///
/// The `Rc`-chain representation makes the *common* case genuinely free:
/// a sibling item at the same depth, or any plain non-marker line,
/// reuses the existing top node via a cheap `Rc::clone()` (no allocation
/// at all), and even a real depth *increase* only ever allocates one
/// small node (not a whole stack) — see [`ListNode::resume_or_push`].
/// `depth` is cached here (rather than derived by walking `parent`) so
/// reading it stays O(1), matching what `Vec::len()` already was for the
/// structure this replaced.
///
/// `PartialEq`/`Eq` are derived, not hand-written, and still do the right
/// thing for `document_cache.rs`'s reconvergence check (which needs two
/// *separately allocated* chains representing the same sequence of
/// indents to compare equal, the same as two `Vec<usize>`s with identical
/// contents always did): `Rc<T>`'s own `PartialEq` impl already compares
/// *values* (`**self == **other`), falling back to that whenever the
/// pointers themselves differ, rather than only ever comparing pointer
/// identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::text_editor) struct ListNode {
    pub(super) indent: usize,
    pub(super) depth: u8,
    pub(super) parent: Option<Rc<ListNode>>,
}

impl ListNode {
    /// Resolves the list-nesting stack's new top node for a marker line at
    /// `indent`, given `current` (the stack's existing top, or `None` if
    /// no list was open) — the structural-sharing replacement for the
    /// original `Vec<usize>`'s "pop while deeper, then push if not already
    /// at this depth" dance. Walks up `parent` links (no allocation) while
    /// the current top is deeper than `indent`; if it lands exactly on
    /// `indent` (a sibling item, or dedenting back to an already-open
    /// ancestor level), that existing node is reused outright — only an
    /// actual depth *increase* allocates, and only one small node.
    pub(super) fn resume_or_push(current: Option<Rc<ListNode>>, indent: usize) -> Rc<ListNode> {
        let mut node = current;
        while let Some(n) = &node {
            if n.indent > indent {
                node = n.parent.clone();
            } else {
                break;
            }
        }
        match node {
            Some(n) if n.indent == indent => n,
            other => {
                let depth = other.as_ref().map_or(1, |n| n.depth + 1);
                Rc::new(ListNode {
                    indent,
                    depth,
                    parent: other,
                })
            }
        }
    }
}

/// `line.trim().is_empty()`, deciding from the first byte when it is a
/// printable ASCII character (nearly every line) before the Unicode-aware
/// trim.
pub(super) fn is_blank(line: &str) -> bool {
    match line.as_bytes().first() {
        None => true,
        Some(&b) if b > b' ' && b < 0x7f => false,
        Some(_) => line.trim().is_empty(),
    }
}

/// The one per-line transition [`classify_block_context`]/
/// [`classify_block_context_from`] both fold over — pulled out of the
/// original single sequential loop the same way `preview::fence::step`/
/// `callout::step` were, so an incremental caller can resume from an
/// arbitrary line's cached entering state instead of only ever starting
/// fresh from line 0.
pub(in crate::text_editor) fn block_context_step(
    open: OpenBlock,
    line: &str,
) -> (LineContext, OpenBlock) {
    if is_blank(line) {
        return (LineContext::Plain, OpenBlock::None);
    }

    if heading_prefix(line).is_some() {
        return (LineContext::Plain, OpenBlock::None);
    }

    if let Some((depth, _)) = blockquote_prefix(line) {
        return (LineContext::Plain, OpenBlock::Blockquote(depth));
    }

    if let Some((_, indent, _)) = indented_list_marker(line) {
        let current = match open {
            OpenBlock::List(top) => Some(top),
            _ => None,
        };
        let top = ListNode::resume_or_push(current, indent);
        let depth = top.depth;
        return (LineContext::ListMarkerLine(depth), OpenBlock::List(top));
    }

    let ctx = match &open {
        OpenBlock::Blockquote(depth) => {
            LineContext::LazyContinuation(LineIndent::Blockquote(*depth))
        }
        OpenBlock::List(top) => LineContext::LazyContinuation(LineIndent::List(top.depth)),
        OpenBlock::None => LineContext::Plain,
    };
    (ctx, open)
}

#[cfg(test)]
/// Resumes classification from `open` (the state *entering* the first line
/// of `source`), which is normally a suffix of the whole document starting
/// at some line, not necessarily the document's actual first line. Returns
/// each line's [`LineContext`] paired with the state *after* it, so an
/// incremental caller can compare that against a previously cached
/// entering state for the corresponding later line to know the moment it's
/// safe to stop and reuse the old cached suffix unchanged — see
/// `document_cache.rs`.
pub(in crate::text_editor) fn classify_block_context_from(
    source: &str,
    open: OpenBlock,
) -> Vec<(LineContext, OpenBlock)> {
    let mut open = open;
    source
        .split('\n')
        .map(|line| {
            let current = std::mem::replace(&mut open, OpenBlock::None);
            let (ctx, next) = block_context_step(current, line);
            open = next.clone();
            (ctx, next)
        })
        .collect()
}
