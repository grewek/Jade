//! Jade's own text editor, built directly on GPUI (not `gpui-component`'s
//! `Editor`/`Textarea`) so Live Preview can change how a block renders
//! without swapping between separate widget instances. See `CLAUDE.md`'s
//! "Custom editor" section for the phased plan this is built in.

mod buffer;
mod editor;
mod element;
mod input_handler;

pub use buffer::EditorBuffer;
pub use editor::{Editor, EditorEvent};
