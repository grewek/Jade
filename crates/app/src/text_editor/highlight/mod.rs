//! Syntax highlighting of fenced code (#99). GPUI-free; nothing here runs in
//! the frame.

mod cache;
mod scope;
mod store;
mod syntax;

// Unused in the test build until the block cache (step 9) consumes them.
#[cfg_attr(test, allow(unused_imports))]
pub(super) use cache::{BlockCache, Limits};
#[cfg_attr(test, allow(unused_imports))]
pub(super) use scope::{role_for_scope, role_for_stack};
#[cfg_attr(test, allow(unused_imports))]
pub(super) use store::Syntaxes;
#[cfg_attr(test, allow(unused_imports))]
pub(super) use syntax::{Highlighted, Highlighter, Loaded, Span};
