//! Jade's own text editor, built directly on GPUI (not `gpui-component`'s
//! `Editor`/`Textarea`) so Live Preview can change how a block renders
//! without swapping between separate widget instances. See `CLAUDE.md`'s
//! "Custom editor" section for the phased plan this is built in.

mod block_preview;
mod buffer;
mod editor;
mod element;
mod input_handler;
mod live_preview;
mod table_preview;

pub use buffer::EditorBuffer;
pub use editor::{Editor, EditorEvent, InsertTab};
