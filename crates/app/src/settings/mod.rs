//! User preferences that change how the editor looks and is laid out (#58).
//! GPUI-free and pure, so it is unit-testable like the theme parser. Colours
//! are the theme engine's business (`crate::theme`); this is sizes, spacing,
//! shapes and switches.

use std::sync::atomic::{AtomicU64, Ordering};

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
