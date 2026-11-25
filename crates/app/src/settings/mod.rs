//! User preferences that change how the editor looks and is laid out (#58).
//! GPUI-free and pure, so it is unit-testable like the theme parser. Colours
//! are the theme engine's business (`crate::theme`); this is sizes, spacing,
//! shapes and switches.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

mod read;
mod typography;

pub use read::{Read, load};

pub use typography::{
    Blockquote, Boxed, BulletShape, Callout, CodeBlock, DoneStyle, Font, FontFamily,
    HEADING_LEVELS, Heading, Image, InlineCode, Link, List, Radius, Shape, Spacing, Table, Task,
    Typography, Underline,
};

/// Bumped whenever a reload changes a layout key. Row counts depend on those
/// keys, so a cache built under another generation is rebuilt; a counter, not
/// a plumbed-through argument, because every row cache would need it.
static LAYOUT_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn layout_generation() -> u64 {
    LAYOUT_GENERATION.load(Ordering::Relaxed)
}

pub fn layout_changed() {
    LAYOUT_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The body font size, row height and math scale in px, as f32 bits. The editor lays out in
/// the render, paint and input phases, which share no style stack to carry
/// them, so they read these (set whenever the typography is) instead of the
/// window's ambient text style. Defaults match [`Typography::default`].
static BODY_SIZE: AtomicU32 = AtomicU32::new(16.0f32.to_bits());
static BODY_ROW: AtomicU32 = AtomicU32::new(26.0f32.to_bits());
static MATH_SCALE: AtomicU32 = AtomicU32::new(1.0f32.to_bits());
static CELL_PADDING: AtomicU32 = AtomicU32::new(0.0f32.to_bits());
static HEADING_SCALE: [AtomicU32; HEADING_LEVELS] =
    [const { AtomicU32::new(1.0f32.to_bits()) }; HEADING_LEVELS];

static HEADING_WEIGHT: [AtomicU32; HEADING_LEVELS] =
    [const { AtomicU32::new(700) }; HEADING_LEVELS];

static HEADING_SPACE: [[AtomicU32; HEADING_LEVELS]; 2] =
    [const { [const { AtomicU32::new(0) }; HEADING_LEVELS] }; 2];
static HEADING_RULE: [AtomicBool; HEADING_LEVELS] =
    [const { AtomicBool::new(false) }; HEADING_LEVELS];

/// The whole body rows left empty above and below a heading of `level`
/// (`heading.space_above`/`space_below`, rounded: row heights stay whole rows,
/// which the row counts and scroll anchoring depend on).
pub fn heading_space_rows(level: u8) -> (usize, usize) {
    let i = Typography::heading_index(level);
    let rows = |side: usize| HEADING_SPACE[side][i].load(Ordering::Relaxed) as usize;
    (rows(0), rows(1))
}

/// `heading.rule` of `level`: a line under the heading.
pub fn heading_rule(level: u8) -> bool {
    HEADING_RULE[Typography::heading_index(level)].load(Ordering::Relaxed)
}

/// `heading.weight` of `level` (1..=6, clamped): the font weight, 100..=900.
pub fn heading_weight(level: u8) -> u16 {
    let i = Typography::heading_index(level);
    HEADING_WEIGHT[i].load(Ordering::Relaxed) as u16
}

/// `heading.scale` of `level` (1..=6, clamped): the heading's font size as a
/// multiple of the body size.
pub fn heading_scale(level: u8) -> f32 {
    let i = Typography::heading_index(level);
    f32::from_bits(HEADING_SCALE[i].load(Ordering::Relaxed))
}

/// How many body rows one row of a heading at `scale` takes: the nearest whole
/// row, at least one. Row heights stay whole editor rows (row counts, scroll
/// anchoring and the estimates depend on that); the text box is 0.75 em, so it
/// fits in the rows this gives.
pub fn heading_rows(scale: f32) -> usize {
    (scale.round().max(1.0)) as usize
}

/// `table.cell_padding` in em: space on each side of a table cell's text on top
/// of the character grid's own.
pub fn cell_padding() -> f32 {
    f32::from_bits(CELL_PADDING.load(Ordering::Relaxed))
}

/// `font.math_scale`: formulas relative to the built-in formula size.
pub fn math_scale() -> f32 {
    f32::from_bits(MATH_SCALE.load(Ordering::Relaxed))
}

pub fn body_font_size() -> f32 {
    f32::from_bits(BODY_SIZE.load(Ordering::Relaxed))
}

pub fn body_row_height() -> f32 {
    f32::from_bits(BODY_ROW.load(Ordering::Relaxed))
}

/// Records the body metrics of `typography`; call whenever the global changes.
pub fn set_body(typography: &Typography) {
    BODY_SIZE.store(typography.font.size.to_bits(), Ordering::Relaxed);
    BODY_ROW.store(typography.line_height_px().to_bits(), Ordering::Relaxed);
    MATH_SCALE.store(typography.font.math_scale.to_bits(), Ordering::Relaxed);
    CELL_PADDING.store(typography.table.cell_padding.to_bits(), Ordering::Relaxed);
    for (slot, scale) in HEADING_SCALE.iter().zip(typography.heading.scale) {
        slot.store(scale.to_bits(), Ordering::Relaxed);
    }
    for (side, spaces) in [
        typography.heading.space_above,
        typography.heading.space_below,
    ]
    .into_iter()
    .enumerate()
    {
        for (slot, space) in HEADING_SPACE[side].iter().zip(spaces) {
            slot.store(space.round().max(0.0) as u32, Ordering::Relaxed);
        }
    }
    for (slot, rule) in HEADING_RULE.iter().zip(typography.heading.rule) {
        slot.store(rule, Ordering::Relaxed);
    }
    for (slot, weight) in HEADING_WEIGHT.iter().zip(typography.heading.weight) {
        slot.store(u32::from(weight), Ordering::Relaxed);
    }
}
