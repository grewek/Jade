use std::ops::Range;

use markdown::ParseOptions;

/// A single top-level Markdown block (paragraph, heading, list, code fence,
/// ...) in a note's source, identified by its byte range. Live Preview swaps
/// each block independently between rendered (`TextView::markdown`) and
/// editable (`Textarea`), depending on which one currently has the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub range: Range<usize>,
}

impl Block {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.clone()]
    }
}

/// Splits `source` into its top-level blocks by byte range, using the same
/// `gfm()` parse options `gpui-component`'s `TextView::markdown` itself parses
/// with — so a boundary found here always matches one it would render.
///
/// This walks the full document AST, which costs the same as a single
/// `markdown::to_mdast` call (see `benches/markdown_parsing.rs`): cheap for a
/// typical note, not free for a worst-case one. Call it when a block is
/// opened, and again once a block is *closed* (its edits are done) to recover
/// from any structural change (a paragraph split in two, a heading demoted),
/// not on every keystroke in between — see [`shift_after_edit`] for that.
pub fn segment_blocks(source: &str) -> Vec<Block> {
    // A single empty block rather than none, so an empty (e.g. brand new)
    // note still has something to activate — there's nothing to click on an
    // empty document, so the caller auto-activates this one instead.
    if source.is_empty() {
        return vec![Block { range: 0..0 }];
    }
    let whole_document = || vec![Block { range: 0..source.len() }];

    let options = ParseOptions::gfm();
    let Ok(root) = markdown::to_mdast(source, &options) else {
        return whole_document();
    };
    let Some(children) = root.children() else {
        return whole_document();
    };

    let blocks: Vec<Block> = children
        .iter()
        .filter_map(|node| node.position())
        .map(|pos| Block { range: pos.start.offset..pos.end.offset })
        .collect();

    if blocks.is_empty() { whole_document() } else { blocks }
}

/// Adjusts `blocks` for a same-structure edit inside `blocks[active]` — typing
/// that changed that block's own length without (as far as this cheap check
/// can tell) splitting, merging, or reclassifying any block — without
/// re-parsing the whole document. `new_len` is the active block's new text
/// length in bytes.
///
/// Only the active block's end and every later block's start/end need to
/// move, by the same constant delta; everything before it, and the blocks'
/// relative order and count, are unaffected by editing text *inside* one of
/// them. This is what keeps typing itself from costing a full re-parse (see
/// [`segment_blocks`]'s doc comment) — it's an O(blocks after the active one)
/// arithmetic fixup, not an O(document) one.
///
/// This does *not* detect structural changes (new paragraph from a blank
/// line, a merged heading, ...) — callers must re-run [`segment_blocks`] once
/// the block is deactivated to correct for those.
pub fn shift_after_edit(blocks: &mut [Block], active: usize, new_len: usize) {
    let Some(old_range) = blocks.get(active).map(|b| b.range.clone()) else {
        return;
    };
    let old_len = old_range.end - old_range.start;
    let delta = new_len as isize - old_len as isize;

    blocks[active].range.end = (old_range.end as isize + delta) as usize;
    for block in &mut blocks[active + 1..] {
        block.range.start = (block.range.start as isize + delta) as usize;
        block.range.end = (block.range.end as isize + delta) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_paragraphs_and_headings_separately() {
        let source = "# Title\n\nFirst paragraph.\n\nSecond paragraph.";
        let blocks = segment_blocks(source);

        let texts: Vec<&str> = blocks.iter().map(|b| b.text(source)).collect();
        assert_eq!(texts, vec!["# Title", "First paragraph.", "Second paragraph."]);
    }

    #[test]
    fn block_ranges_cover_their_own_text_exactly_and_dont_overlap() {
        let source = "# Title\n\nFirst paragraph.\n\nSecond paragraph.";
        let blocks = segment_blocks(source);

        for window in blocks.windows(2) {
            assert!(window[0].range.end <= window[1].range.start);
        }
        for block in &blocks {
            assert_eq!(&source[block.range.clone()], block.text(source));
        }
    }

    #[test]
    fn a_list_is_a_single_block() {
        let source = "- one\n- two\n- three";
        let blocks = segment_blocks(source);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text(source), source);
    }

    #[test]
    fn empty_source_has_a_single_empty_block() {
        assert_eq!(segment_blocks(""), vec![Block { range: 0..0 }]);
    }

    #[test]
    fn shift_after_edit_grows_the_active_block_and_moves_later_ones() {
        let source = "First paragraph.\n\nSecond paragraph.";
        let mut blocks = segment_blocks(source);
        let second_start_before = blocks[1].range.start;

        // Simulate typing 5 more characters into the first (active) block.
        let new_len = blocks[0].range.len() + 5;
        shift_after_edit(&mut blocks, 0, new_len);

        assert_eq!(blocks[0].range.end, "First paragraph.".len() + 5);
        assert_eq!(blocks[1].range.start, second_start_before + 5);
        assert_eq!(blocks[1].range.len(), "Second paragraph.".len());
    }

    #[test]
    fn shift_after_edit_shrinks_the_active_block_and_moves_later_ones_back() {
        let source = "First paragraph.\n\nSecond paragraph.";
        let mut blocks = segment_blocks(source);
        let second_start_before = blocks[1].range.start;

        let new_len = blocks[0].range.len() - 3;
        shift_after_edit(&mut blocks, 0, new_len);

        assert_eq!(blocks[0].range.end, "First paragraph.".len() - 3);
        assert_eq!(blocks[1].range.start, second_start_before - 3);
    }

    #[test]
    fn shift_after_edit_on_the_last_block_moves_nothing_else() {
        let source = "First paragraph.\n\nSecond paragraph.";
        let mut blocks = segment_blocks(source);
        let first_range_before = blocks[0].range.clone();

        let new_len = blocks[1].range.len() + 10;
        shift_after_edit(&mut blocks, 1, new_len);

        assert_eq!(blocks[0].range, first_range_before);
        assert_eq!(blocks[1].range.end, blocks[1].range.start + "Second paragraph.".len() + 10);
    }
}
