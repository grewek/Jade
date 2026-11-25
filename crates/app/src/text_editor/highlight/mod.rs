//! Syntax highlighting of fenced code (#99). GPUI-free; nothing here runs in
//! the frame.

mod scope;

pub(super) use scope::{role_for_scope, role_for_stack};
