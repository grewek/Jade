//! LaTeX math (`$…$`, `$$…$$`): parsing a formula's source into a tree. GPUI-free
//! like the rest of the pure editor logic; layout and painting build on the
//! tree and the bundled Latin Modern Math font (#13).

// The renderer that uses it follows in #13; until then only tests do.
#![allow(dead_code, unused_imports)]

mod alphabet;
mod ast;
mod font;
mod layout;
#[cfg(test)]
mod layout_tests;
mod lex;
mod parse;
mod symbols;
#[cfg(test)]
mod tests;

pub use ast::*;
pub use font::MathFont;
pub use layout::{Item, Layout, layout};
pub use parse::{Formula, MAX_DEPTH, MAX_SOURCE, parse};
