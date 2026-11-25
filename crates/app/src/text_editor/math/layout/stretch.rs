//! Glyphs grown to a length: the smallest size variant that is long enough,
//! else the font's parts laid end to end (OpenType MATH glyph assembly).

use ttf_parser::GlyphId;

use super::super::font::{Axis, Part};
use super::{Item, Layout, Typesetter};

/// Most times an extender part repeats; bounds the work for absurd lengths.
const MAX_REPEATS: usize = 64;

/// A horizontally stretched glyph. `layout` starts at x = 0 on the glyph's own
/// baseline; `top` and `bottom` are the ink's edges above that baseline in em
/// (both positive for a mark that floats above it).
pub(super) struct Stretched {
    pub layout: Layout,
    pub top: f32,
    pub bottom: f32,
}

/// Where each part of an assembly starts along the axis, and the total length.
/// Parts that are `extender`s repeat as often as the target needs; connected
/// parts overlap by as much as the font allows, at least `min_overlap`, so the
/// total hits `target` when it can (it never goes below the shortest form).
pub(super) fn assemble(parts: &[Part], min_overlap: f32, target: f32) -> (Vec<(usize, f32)>, f32) {
    assert!(!parts.is_empty());
    let repeats = repeats_for(parts, min_overlap, target);
    let mut seq: Vec<usize> = Vec::with_capacity(parts.len() * repeats);
    for (i, part) in parts.iter().enumerate() {
        let copies = if part.extender { repeats } else { 1 };
        seq.extend(std::iter::repeat_n(i, copies));
    }
    let longest = |i: usize| {
        let (a, b) = (&parts[seq[i]], &parts[seq[i + 1]]);
        a.end_overlap.min(b.start_overlap).max(min_overlap)
    };
    let joints = seq.len() - 1;
    let advance: f32 = seq.iter().map(|&i| parts[i].advance).sum();
    let (most, least): (f32, f32) = ((0..joints).map(longest).sum(), joints as f32 * min_overlap);
    // Total overlap that makes the length `target`, within what is allowed.
    let wanted = (advance - target).clamp(least.min(most), most);
    let t = if most > least {
        (wanted - least) / (most - least)
    } else {
        0.0
    };
    let mut placed = Vec::with_capacity(seq.len());
    let mut at = 0.0;
    for (n, &i) in seq.iter().enumerate() {
        placed.push((i, at));
        at += parts[i].advance;
        if n < joints {
            at -= min_overlap + t * (longest(n) - min_overlap);
        }
    }
    (placed, at)
}

/// The fewest copies of each extender (at least one) that can reach `target`.
fn repeats_for(parts: &[Part], min_overlap: f32, target: f32) -> usize {
    let fixed: (f32, usize) = parts
        .iter()
        .filter(|p| !p.extender)
        .fold((0.0, 0), |(len, n), p| (len + p.advance, n + 1));
    let spare: (f32, usize) = parts
        .iter()
        .filter(|p| p.extender)
        .fold((0.0, 0), |(len, n), p| (len + p.advance, n + 1));
    if spare.1 == 0 {
        return 1;
    }
    (1..=MAX_REPEATS)
        .find(|&r| {
            let joints = (fixed.1 + r * spare.1).saturating_sub(1);
            let length = fixed.0 + r as f32 * spare.0 - joints as f32 * min_overlap;
            length >= target
        })
        .unwrap_or(MAX_REPEATS)
}

impl Typesetter<'_> {
    /// `base` stretched to `target` em across, at `size`: a variant if one is
    /// long enough, else an assembly, else the longest variant there is.
    pub(super) fn stretch_wide(&self, base: GlyphId, target: f32, size: f32) -> Stretched {
        let unit = target / size;
        let variants = self.font.variants(base, Axis::Horizontal);
        let fitting = variants.iter().find(|(_, len)| *len >= unit);
        if let Some(&(id, _)) = fitting {
            return self.wide_glyph(id, size);
        }
        let parts = self.font.assembly(base, Axis::Horizontal);
        if parts.is_empty() {
            return self.wide_glyph(variants.last().map_or(base, |v| v.0), size);
        }
        let (placed, total) = assemble(&parts, self.font.min_overlap(), unit);
        let mut out = Layout::default();
        let (mut top, mut bottom) = (f32::MIN, f32::MAX);
        for (i, at) in placed {
            let m = self.font.metrics(parts[i].id);
            top = top.max(m.ascent);
            bottom = bottom.min(-m.descent);
            out.items.push(Item::Glyph {
                id: parts[i].id.0,
                x: at * size,
                y: 0.0,
                size,
                error: false,
                color: None,
            });
        }
        out.width = total * size;
        out.ascent = top.max(0.0) * size;
        out.descent = (-bottom).max(0.0) * size;
        Stretched {
            layout: out,
            top: top * size,
            bottom: bottom * size,
        }
    }

    /// One glyph with its ink starting at x = 0 (accent marks are drawn left
    /// of their origin).
    fn wide_glyph(&self, id: GlyphId, size: f32) -> Stretched {
        let m = self.font.metrics(id);
        let (left, right) = self.font.ink_x(id);
        let mut layout = self.glyph(id, size, false);
        layout.items = vec![Item::Glyph {
            id: id.0,
            x: -left * size,
            y: 0.0,
            size,
            error: false,
            color: None,
        }];
        layout.width = (right - left) * size;
        layout.italic = 0.0;
        Stretched {
            layout,
            top: m.ascent * size,
            bottom: -m.descent * size,
        }
    }

    /// A glyph built from parts to `target` em tall at `size`, its origin on
    /// the bottom edge; `None` if the font gives the glyph no assembly.
    pub(super) fn assemble_tall(&self, base: GlyphId, target: f32, size: f32) -> Option<Layout> {
        let parts = self.font.assembly(base, Axis::Vertical);
        if parts.is_empty() {
            return None;
        }
        let (placed, total) = assemble(&parts, self.font.min_overlap(), target / size);
        let mut out = Layout::default();
        for (i, at) in placed {
            out.width = out.width.max(self.font.metrics(parts[i].id).advance * size);
            out.items.push(Item::Glyph {
                id: parts[i].id.0,
                x: 0.0,
                y: -at * size,
                size,
                error: false,
                color: None,
            });
        }
        out.ascent = total * size;
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(advance: f32, extender: bool) -> Part {
        let overlap = if extender { 0.4 } else { 0.2 };
        Part {
            id: GlyphId(0),
            start_overlap: overlap,
            end_overlap: overlap,
            advance,
            extender,
        }
    }

    #[test]
    fn an_assembly_hits_its_target_length() {
        let parts = [part(1.0, false), part(0.5, true), part(1.0, false)];
        for target in [2.2, 3.0, 5.0, 9.0] {
            let (placed, total) = assemble(&parts, 0.02, target);
            assert!((total - target).abs() < 1e-3, "{target}: {total}");
            assert!(placed.windows(2).all(|w| w[0].1 < w[1].1), "{placed:?}");
        }
    }

    #[test]
    fn an_assembly_never_goes_below_its_shortest_form() {
        let parts = [part(1.0, false), part(0.5, true), part(1.0, false)];
        let (_, total) = assemble(&parts, 0.02, 0.5);
        assert!(total >= 2.0, "{total}");
    }

    #[test]
    fn the_repeat_count_is_bounded() {
        let parts = [part(1.0, false), part(0.5, true)];
        let (placed, _) = assemble(&parts, 0.02, 1e9);
        assert!(placed.len() <= 1 + MAX_REPEATS);
    }
}
