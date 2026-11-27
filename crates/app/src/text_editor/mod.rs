//! Jade's own text editor, built directly on GPUI (not `gpui-component`'s
//! `Editor`/`Textarea`) so Live Preview can change how a block renders
//! without swapping between separate widget instances. See `CLAUDE.md`'s
//! "Custom editor" section for the phased plan this is built in.

pub mod bench_api;
mod buffer;
mod colors;
mod columns;
mod doc;
mod doc_stats;
mod editor;
mod element;
mod headings;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "only tests use the highlighter until the block cache (#99 step 9)"
    )
)]
mod highlight;
mod image;
mod input_handler;
mod layout;
mod lines;
mod link_table;
mod math;
#[cfg(any(test, feature = "debug-socket"))]
mod paint_trace;
#[cfg(not(any(test, feature = "debug-socket")))]
#[path = "paint_trace_stub.rs"]
mod paint_trace;
mod preview;
mod rows;
mod table;
#[cfg(test)]
mod test_support;

pub mod settings;

pub use buffer::EditorBuffer;
#[cfg(feature = "bench-support")]
pub use editor::{BackTab, InsertTab};
pub use editor::{Editor, EditorEvent, key_bindings};
#[cfg(test)]
pub(crate) use paint_trace::{PaintedFrame, PaintedLine, last_painted_frame};
pub use settings::TextSettings;
