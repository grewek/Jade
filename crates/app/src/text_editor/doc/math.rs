//! The `$$` blocks of the note, as line ranges. A block is math, not Markdown:
//! every pass that reads prose (links, headings, embeds) skips its lines.
//!
//! An edit rescans a bounded window, like tables: a block closes within
//! [`MAX_BLOCK_LINES`] lines of its opener, so only blocks that start that far
//! above the edit, or that the edit touched, can change.

use jade_core::math_scan::{MAX_BLOCK_LINES, scan_blocks};

use super::*;

/// A `$$` block: the lines from its opening `$$` to its closing one, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::text_editor) struct MathLines {
    pub first_line: usize,
    pub last_line: usize,
}

/// What `patch_math` must read from the *old* layout before the line shift
/// destroys it.
pub(super) struct MathPre {
    /// First line (old numbering, which equals new before the edit) and one
    /// past the last line (new numbering) of the old blocks the edit touched.
    extent: Option<(usize, usize)>,
}

impl DocumentCache {
    pub(in crate::text_editor) fn math_blocks(&self) -> &[MathLines] {
        &self.math_blocks
    }

    /// The `$$` block that has `line`, if any.
    pub(in crate::text_editor) fn math_block_at(&self, line: usize) -> Option<&MathLines> {
        let i = self.math_blocks.partition_point(|b| b.last_line < line);
        self.math_blocks.get(i).filter(|b| b.first_line <= line)
    }

    /// Whether `line` is inside a `$$` block (its `$$` lines included).
    pub(in crate::text_editor) fn in_math_block(&self, line: usize) -> bool {
        !self.math_blocks.is_empty() && self.math_block_at(line).is_some()
    }

    pub(super) fn recompute_math(&mut self, text: &str) {
        self.math_blocks.clear();
        if !text.contains("$$") {
            return;
        }
        self.math_blocks
            .extend(scan_blocks(text).into_iter().map(|b| MathLines {
                first_line: b.first_line,
                last_line: b.last_line,
            }));
    }

    /// Drops the blocks the edit touched and shifts the later ones, saving
    /// what [`Self::patch_math`] needs. Runs before [`Self::resize_lines`].
    pub(super) fn math_pre(&mut self, span: &EditSpan) -> MathPre {
        let old_last = span.start_line + span.lines_removed;
        let first = self
            .math_blocks
            .partition_point(|b| b.last_line < span.start_line);
        let end = self
            .math_blocks
            .partition_point(|b| b.first_line <= old_last);
        let end = end.max(first);
        let delta = span.line_delta();
        let extent = (first < end).then(|| {
            let start = self.math_blocks[first].first_line;
            let last = self.math_blocks[end - 1].last_line;
            // May land before the edit for a block inside a deleted range.
            (start, (last as isize + 1 + delta).max(0) as usize)
        });
        self.math_blocks.drain(first..end);
        for b in &mut self.math_blocks[first..] {
            b.first_line = shift_line(b.first_line, delta);
            b.last_line = shift_line(b.last_line, delta);
        }
        MathPre { extent }
    }

    /// Re-derives the blocks the edit can have changed from a window of the new
    /// text and splices them in. Returns the lines whose block membership may
    /// have changed.
    pub(super) fn patch_math(
        &mut self,
        text: &str,
        span: &EditSpan,
        pre: &MathPre,
        touched_roles: &Range<usize>,
    ) -> Range<usize> {
        let line_count = self.line_roles.len();
        let last_new = span.last_new_line().min(line_count - 1);
        let minimal = span.start_line..last_new + 1;
        // Code lines are never math, and nothing was a block: an edit inside a
        // fence changes nothing here.
        let all_code = touched_roles
            .clone()
            .chain(minimal.clone())
            .all(|i| !matches!(self.line_roles.get(i), Some(LineRole::Plain)));
        // A fence opened above turns lines code that were math: blocks inside
        // the lines whose role changed must go.
        let covered = self
            .math_blocks
            .partition_point(|b| b.last_line < touched_roles.start)
            < self
                .math_blocks
                .partition_point(|b| b.first_line < touched_roles.end);
        if pre.extent.is_none() && all_code && !covered {
            return minimal;
        }

        let mut ws = span.start_line.saturating_sub(MAX_BLOCK_LINES);
        if let Some((start, _)) = pre.extent {
            ws = ws.min(start);
        }
        ws = ws.min(touched_roles.start);
        // The scan needs a start outside any code fence.
        let is_open = |s: &Self, i: usize| s.fence_states[i] != preview::fence::pack_fence(None);
        if is_open(self, span.start_line.min(line_count - 1)) {
            // The edit is inside what is (now) a fence: back to its start.
            while ws > 0 && is_open(self, ws) {
                ws -= 1;
            }
        }
        while ws < span.start_line && is_open(self, ws) {
            ws += 1;
        }
        // A block that began above the window and still covers its start.
        if let Some(b) = self.math_block_at(ws).copied() {
            ws = b.first_line;
        }

        // A fence opened or closed changes what is code (so not math) far below.
        let edited_end = (last_new + 1).max(touched_roles.end).min(line_count);
        let mut we = (edited_end + MAX_BLOCK_LINES).min(line_count);
        let (mut local, mut cut);
        loop {
            let from = self.line_start_byte(ws);
            let to = self.line_end_byte(we - 1);
            local = scan_blocks(&text[from..to]);
            // Everything before `cut` is re-derived; blocks from `cut` on
            // are the untouched old ones.
            cut = edited_end.max(pre.extent.map_or(0, |(_, end)| end));
            loop {
                let before = cut;
                for b in &local {
                    if ws + b.first_line < cut {
                        cut = cut.max(ws + b.last_line + 1);
                    }
                }
                let i = self.math_blocks.partition_point(|b| b.last_line < ws);
                for b in &self.math_blocks[i..] {
                    if b.first_line >= cut {
                        break;
                    }
                    cut = cut.max(b.last_line + 1);
                }
                if cut == before {
                    break;
                }
            }
            if we == line_count || cut + MAX_BLOCK_LINES <= we {
                break;
            }
            we = (cut + MAX_BLOCK_LINES).min(line_count);
        }
        let first = self.math_blocks.partition_point(|b| b.first_line < ws);
        let end = self
            .math_blocks
            .partition_point(|b| b.first_line < cut)
            .max(first);
        let fresh: Vec<MathLines> = local
            .into_iter()
            .map(|b| MathLines {
                first_line: ws + b.first_line,
                last_line: ws + b.last_line,
            })
            .filter(|b| b.first_line < cut)
            .collect();
        // What to relayout: the edit, the blocks it touched (a block is laid
        // out as a whole) and every block that appeared or went away. The
        // window itself is wider than that.
        let mut touched = minimal;
        let mut cover = |first_line: usize, last_line: usize| {
            touched = union_ranges(touched.clone(), first_line..last_line + 1);
        };
        if let Some((start, end)) = pre.extent {
            cover(start, end.max(start + 1) - 1);
        }
        let old = &self.math_blocks[first..end];
        for b in old.iter().filter(|b| !fresh.contains(b)) {
            cover(b.first_line, b.last_line);
        }
        for b in &fresh {
            if !old.contains(b) || (b.first_line < last_new + 1 && b.last_line >= span.start_line) {
                cover(b.first_line, b.last_line);
            }
        }
        self.math_blocks.splice(first..end, fresh);
        touched
    }
}
