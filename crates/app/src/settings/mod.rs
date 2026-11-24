//! User preferences that change how the editor looks and is laid out (#58).
//! GPUI-free and pure, so it is unit-testable like the theme parser. Colours
//! are the theme engine's business (`crate::theme`); this is sizes, spacing,
//! shapes and switches.

mod read;
mod typography;

pub use read::{Read, load};

pub use typography::{
    Blockquote, Boxed, BulletShape, Callout, CodeBlock, DoneStyle, Font, FontFamily,
    HEADING_LEVELS, Heading, Image, InlineCode, Link, List, Radius, Shape, Spacing, Table, Task,
    Typography, Underline,
};
