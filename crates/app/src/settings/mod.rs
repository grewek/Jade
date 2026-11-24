//! User preferences that change how the editor looks and is laid out (#58).
//! GPUI-free and pure, so it is unit-testable like the theme parser. Colours
//! are the theme engine's business (`crate::theme`); this is sizes, spacing,
//! shapes and switches.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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
}
