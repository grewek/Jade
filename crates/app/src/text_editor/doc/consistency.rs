//! Whether the edit-patched classification cache still equals one built from
//! the text, for the cache's tests and the debug socket's invariants query.

use super::DocumentCache;

impl DocumentCache {
    /// The names of the parts that differ from a cache built from `text`.
    /// Every part counts, also the entering states and the line index: a stale
    /// entry there could go unseen until a later edit resumes from it. Folding
    /// a callout is view state, so it may differ. Builds a whole cache.
    pub(in crate::text_editor) fn stale_fields(&self, text: &str) -> Vec<&'static str> {
        let fresh = Self::new(text);
        let lines = fresh.line_index.len();
        let callouts_equal = self.callouts.len() == fresh.callouts.len()
            && self.callouts.iter().zip(&fresh.callouts).all(|(a, b)| {
                let mut a = a.clone();
                a.folded = b.folded;
                a == *b
            });
        let line_index_equal = self.line_index.len() == lines
            && (0..=lines).all(|i| self.line_index.prefix_sum(i) == fresh.line_index.prefix_sum(i));
        [
            ("line roles", self.line_roles == fresh.line_roles),
            ("fence states", self.fence_states == fresh.fence_states),
            (
                "block contexts",
                self.block_contexts == fresh.block_contexts,
            ),
            (
                "open block states",
                self.open_block_states == fresh.open_block_states,
            ),
            ("tables", self.tables == fresh.tables),
            ("math blocks", self.math_blocks == fresh.math_blocks),
            ("macro lines", self.macro_lines == fresh.macro_lines),
            ("macros", self.macros.stamp() == fresh.macros.stamp()),
            ("callouts", callouts_equal),
            (
                "callout states",
                self.callout_states == fresh.callout_states,
            ),
            (
                "column markers",
                self.column_markers == fresh.column_markers,
            ),
            ("columns", self.columns == fresh.columns),
            ("line index", line_index_equal),
        ]
        .into_iter()
        .filter_map(|(name, equal)| (!equal).then_some(name))
        .collect()
    }
}
