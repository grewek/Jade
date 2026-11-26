//! The scroll position as `(block, offset in block)`. Moving it and clamping
//! it read only the heights of the blocks it passes, never a total, so
//! nothing above the anchor can move what is on screen.

use super::heights::HeightMap;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct Anchor {
    pub(crate) block: usize,
    /// In `0.0..height(block)`.
    pub(crate) offset: f64,
}

impl Anchor {
    /// Moves down by `delta` px (up if negative), stopping at both ends.
    pub(crate) fn moved(self, delta: f64, heights: &HeightMap) -> Self {
        let Self {
            mut block,
            mut offset,
        } = self;
        offset += delta;
        while offset < 0. && block > 0 {
            block -= 1;
            offset += heights.height(block);
        }
        while block + 1 < heights.len() && offset >= heights.height(block) {
            offset -= heights.height(block);
            block += 1;
        }
        let offset = offset.clamp(0., heights.height(block));
        Self { block, offset }
    }

    /// Keeps the anchor valid: inside its block, and with a full viewport of
    /// content below it unless the document is shorter than that.
    pub(crate) fn clamped(self, viewport: f64, heights: &HeightMap) -> Self {
        let block = self.block.min(heights.len() - 1);
        // A block that shrank under the offset leaves the anchor at its end.
        let offset = self.offset.clamp(0., heights.height(block));
        let inside = Self { block, offset }.moved(0., heights);
        let mut below = -inside.offset;
        let mut next = inside.block;
        while next < heights.len() && below < viewport {
            below += heights.height(next);
            next += 1;
        }
        if below < viewport {
            inside.moved(below - viewport, heights)
        } else {
            inside
        }
    }

    /// The content y of the anchor. Only for the scrollbar.
    pub(crate) fn content_y(self, heights: &HeightMap) -> f64 {
        heights.before(self.block) + self.offset
    }

    pub(crate) fn at(y: f64, heights: &HeightMap) -> Self {
        let (block, offset) = heights.find(y);
        Self { block, offset }
    }

    /// The least movement that shows `block` (its top if it is taller than
    /// the viewport): what a caret or "go to heading" needs.
    pub(crate) fn revealing(self, block: usize, viewport: f64, heights: &HeightMap) -> Self {
        let height = heights.height(block);
        let top = Self { block, offset: 0. };
        if block <= self.block || height >= viewport {
            return top;
        }
        // The topmost position that still shows the whole block.
        let lowest = top.moved(height - viewport, heights);
        let below = (lowest.block, lowest.offset) > (self.block, self.offset);
        if below { lowest } else { self }
    }
}
