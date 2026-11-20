//! Jade's own text editor, built directly on GPUI (not `gpui-component`'s
//! `Editor`/`Textarea`) so Live Preview can change how a block renders
//! without swapping between separate widget instances. See `CLAUDE.md`'s
//! "Custom editor" section for the phased plan this is built in.

pub mod bench_api;
mod block;
mod block_preview;
mod buffer;
mod callout;
mod colors;
mod doc_stats;
mod document_cache;
mod editor;
mod element;
mod fenwick;
mod headings;
mod image_cache;
mod images;
mod input_handler;
mod line_chunks;
mod link_table;
mod live_preview;
#[cfg(any(test, feature = "debug-socket"))]
mod paint_trace;
#[cfg(not(any(test, feature = "debug-socket")))]
#[path = "paint_trace_stub.rs"]
mod paint_trace;
mod table_box;
mod table_handles;
mod table_ops;
mod table_preview;
#[cfg(test)]
mod test_support;

pub use buffer::EditorBuffer;
pub use editor::{Editor, EditorEvent, key_bindings};
