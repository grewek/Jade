//! The narrow surface criterion benches use (`benches/editor.rs`); everything
//! else in `text_editor` stays crate-private. Wrappers return small summaries
//! so the optimizer cannot discard the work being timed.

use super::doc::{DocumentCache, EditPatch, TextEdit};
use super::doc_stats::DocStats;
use super::lines::Fenwick;
use super::lines::line_strides as strides_of;
use super::link_table::LinkTable;

pub use super::buffer::EditorBuffer;

/// Per-line byte strides of `text`: the one newline scan of a cold build,
/// which the passes below then read their lines from.
pub fn line_strides(text: &str) -> Vec<i64> {
    strides_of(text)
}

/// An edit that alternately inserts `insert` at `at` and removes it again, so
/// a bench loop returns the document to its start state every second step.
pub struct Toggle {
    at: usize,
    insert: &'static str,
    inserted: bool,
}

impl Toggle {
    pub fn new(at: usize, insert: &'static str) -> Self {
        Self {
            at,
            insert,
            inserted: false,
        }
    }

    /// Applies the next step to `text`; returns `(at, old_end, new_end)`, the
    /// arguments the `edit`/`record_edit` calls take.
    pub fn apply(&mut self, text: &mut String) -> (usize, usize, usize) {
        let (at, len) = (self.at, self.insert.len());
        let (old_end, new_end) = if self.inserted {
            text.replace_range(at..at + len, "");
            (at + len, at)
        } else {
            text.insert_str(at, self.insert);
            (at, at + len)
        };
        self.inserted = !self.inserted;
        (at, old_end, new_end)
    }
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

    pub fn recompute_columns(&mut self, text: &str, strides: &[i64]) {
        self.0.recompute_columns(text, strides);
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

/// Word/character counting: `build` is the cold pass at note open; `edit`
/// then `count` split one edit into the classification patch (timed
/// elsewhere) and the status-bar patch being measured.
pub struct Counts {
    cache: DocumentCache,
    stats: DocStats,
    patch: Option<EditPatch>,
}

impl Counts {
    pub fn new(text: &str) -> Self {
        Self {
            cache: DocumentCache::new(text),
            stats: DocStats::new(text),
            patch: None,
        }
    }

    /// A cold count of the whole document: (words, characters).
    pub fn build(text: &str) -> (usize, usize) {
        let stats = DocStats::new(text);
        (stats.words(), stats.chars())
    }

    pub fn edit(
        &mut self,
        text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) {
        let edit = TextEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
        };
        self.patch = self.cache.record_edit(text, edit);
    }

    /// Updates the counts for the edit last passed to [`Self::edit`].
    pub fn count(&mut self, text: &str) -> (usize, usize) {
        match self.patch.take() {
            Some(patch) => self.stats.patch(text, &self.cache, &patch),
            None => self.stats.rebuild(text),
        }
        (self.stats.words(), self.stats.chars())
    }
}

/// The link table: `rebuild` is the cold scan at note open (the
/// classification cache it reads roles from is built outside the timing);
/// `edit` then `patch` split one edit into the cache patch and the table
/// patch being measured.
pub struct Links {
    cache: DocumentCache,
    table: LinkTable,
    patch: Option<EditPatch>,
}

impl Links {
    pub fn new(text: &str) -> Self {
        let cache = DocumentCache::new(text);
        let table = LinkTable::new(text, &cache);
        Self {
            cache,
            table,
            patch: None,
        }
    }

    /// Scans the whole document again; returns the number of links.
    pub fn rebuild(&mut self, text: &str) -> usize {
        self.table.rebuild(text, &self.cache);
        self.table.len()
    }

    pub fn edit(
        &mut self,
        text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) {
        let edit = TextEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
        };
        self.patch = self.cache.record_edit(text, edit);
    }

    /// Updates the table for the edit last passed to [`Self::edit`].
    pub fn patch(&mut self, text: &str) -> usize {
        match self.patch.take() {
            Some(patch) => self.table.patch(text, &self.cache, &patch),
            None => self.table.rebuild(text, &self.cache),
        }
        self.table.len()
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

/// Fenced-code highlighting over a folder of definitions (#99).
pub struct Syntaxes(super::highlight::Highlighter);

impl Syntaxes {
    /// Loads the folder; the count is how many files gave a notice.
    pub fn load(dir: &std::path::Path) -> (Self, usize) {
        let loaded = super::highlight::Highlighter::load_dir(dir);
        (Self(loaded.highlighter), loaded.notices.len())
    }

    /// The block cache over the same folder, for the per-block lookups.
    pub fn into_cache(self) -> BlockSpans {
        BlockSpans(super::highlight::BlockCache::new(
            self.0,
            super::highlight::Limits::default(),
        ))
    }

    /// The number of coloured spans in `text`, or 0 for an unknown language.
    pub fn highlight(&self, language: &str, text: &str) -> usize {
        self.0
            .highlight(language, text)
            .map_or(0, |h| h.lines.iter().map(Vec::len).sum())
    }
}

/// [`Syntaxes`] behind the per-block cache (#99).
pub struct BlockSpans(super::highlight::BlockCache);

impl BlockSpans {
    /// The number of coloured spans of the block, 0 when it is plain.
    pub fn spans(&mut self, language: &str, text: &str) -> usize {
        self.0
            .spans(language, text)
            .map_or(0, |h| h.lines.iter().map(Vec::len).sum())
    }
}
