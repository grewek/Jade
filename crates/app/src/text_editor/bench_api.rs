//! The narrow surface criterion benches use (`benches/editor.rs`); everything
//! else in `text_editor` stays crate-private. Wrappers return small summaries
//! so the optimizer cannot discard the work being timed.

use super::document_cache::{DocumentCache, TextEdit, line_strides as strides_of};
use super::fenwick::Fenwick;

pub use super::buffer::EditorBuffer;

/// Per-line byte strides of `text`: the one newline scan of a cold build,
/// which the passes below then read their lines from.
pub fn line_strides(text: &str) -> Vec<i64> {
    strides_of(text)
}

/// The per-document classification cache every keystroke patches.
pub struct Cache(DocumentCache);

impl Cache {
    pub fn new(text: &str) -> Self {
        Self(DocumentCache::new(text))
    }

    pub fn line_count(&self) -> usize {
        self.0.line_count()
    }

    pub fn line_start_byte(&self, line: usize) -> usize {
        self.0.line_start_byte(line)
    }

    /// One of the four classification passes over `text`, as run by a cold
    /// build; the other three are left as they were.
    pub fn recompute_line_roles(&mut self, text: &str, strides: &[i64]) {
        self.0.recompute_block_preview(text, strides);
    }

    pub fn recompute_callouts(&mut self, text: &str, strides: &[i64]) {
        self.0.recompute_callouts(text, strides);
    }

    pub fn recompute_block_contexts(&mut self, text: &str, strides: &[i64]) {
        self.0.recompute_block_context(text, strides);
    }

    pub fn recompute_tables(&mut self, text: &str, strides: &[i64]) {
        self.0.recompute_tables(text, strides);
    }

    /// Reports an edit already applied to `text`; `true` when the cache was
    /// patched incrementally, `false` when it fell back to a full recompute.
    pub fn record_edit(
        &mut self,
        text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) -> bool {
        let edit = TextEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
        };
        self.0.record_edit(text, edit).is_some()
    }
}

/// The Fenwick tree behind line-start and row-height lookups.
pub struct Index(Fenwick);

impl Index {
    pub fn new(values: &[i64]) -> Self {
        Self(Fenwick::new(values))
    }

    pub fn prefix_sum(&self, i: usize) -> i64 {
        self.0.prefix_sum(i)
    }

    pub fn add_at(&mut self, i: usize, delta: i64) {
        self.0.add_at(i, delta);
    }

    pub fn find(&self, target: i64) -> Option<usize> {
        self.0.find(target)
    }
}
