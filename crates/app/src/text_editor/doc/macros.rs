//! The macros a note defines (#14): the lines that hold a definition and the
//! table they make, which every formula of the note is expanded with.
//!
//! An edit looks only at the lines it changed (and at the `$$` block they are
//! in): the table is rebuilt from the few definition lines when one of them
//! is touched or a new one appears, and the whole note's formulas are then
//! laid out again (see `record_edit`).

use std::sync::Arc;

use jade_core::math_scan::scan_inline;

use super::*;
use crate::text_editor::math::{Macros, definition_starts, may_define, span_body};

impl DocumentCache {
    /// The note's macros; empty for nearly every note.
    pub(in crate::text_editor) fn macros(&self) -> &Macros {
        &self.macros
    }

    /// Finds every definition line of `text` and rebuilds the table. Whether
    /// the table changed.
    pub(super) fn recompute_macros(&mut self, text: &str) -> bool {
        self.macro_lines.clear();
        let mut last = None;
        for at in definition_starts(text) {
            let line = self.line_at(at);
            if last != Some(line) && self.is_plain(line) {
                self.macro_lines.push(line);
            }
            last = Some(line);
        }
        self.rebuild_macros(text)
    }

    /// Keeps the definition lines in step with an edit and rebuilds the table
    /// if a definition was touched. `touched` is every line any classifier
    /// re-derived (a closed fence far above can turn code lines into text).
    /// Whether the table changed. Runs after `patch_math`, so the blocks have
    /// their new numbering.
    pub(super) fn patch_macros(
        &mut self,
        text: &str,
        span: &EditSpan,
        touched: &Range<usize>,
    ) -> bool {
        let old_last = span.start_line + span.lines_removed;
        let delta = span.line_delta();
        // The old lines the edit replaced are gone; the later ones move.
        let before = self.macro_lines.len();
        self.macro_lines
            .retain(|&l| l < span.start_line || l > old_last);
        let gone = self.macro_lines.len() != before;
        for l in &mut self.macro_lines {
            if *l > old_last {
                *l = shift_line(*l, delta);
            }
        }
        let edited = union_ranges(touched.clone(), span.start_line..span.last_new_line() + 1);
        let edited = edited.start..edited.end.min(self.line_roles.len());
        let (first, end) = (
            self.macro_lines.partition_point(|&l| l < edited.start),
            self.macro_lines.partition_point(|&l| l < edited.end),
        );
        let lost = gone || first < end;
        self.macro_lines.drain(first..end);
        let mut found = Vec::new();
        if may_define(self.edited_text(text, &edited)) {
            for i in edited.clone() {
                let line = &text[self.line_start_byte(i)..self.line_end_byte(i)];
                if may_define(line) && self.is_plain(i) {
                    found.push(i);
                }
            }
        }
        let added = !found.is_empty();
        self.macro_lines.splice(first..first, found);
        // A definition may span the lines of a `$$` block the edit is in.
        let inside = self.macro_lines.iter().any(|&l| {
            self.math_block_at(l)
                .is_some_and(|b| b.first_line < edited.end && b.last_line >= edited.start)
        });
        if !(lost || added || inside) {
            return false;
        }
        self.rebuild_macros(text)
    }

    fn edited_text<'t>(&self, text: &'t str, lines: &Range<usize>) -> &'t str {
        if lines.is_empty() {
            return "";
        }
        &text[self.line_start_byte(lines.start)..self.line_end_byte(lines.end - 1)]
    }

    /// Whether line `i` is plain text, not code: only there is a definition one.
    fn is_plain(&self, i: usize) -> bool {
        matches!(self.line_roles.get(i), Some(LineRole::Plain))
    }

    fn rebuild_macros(&mut self, text: &str) -> bool {
        let mut sources: Vec<&str> = Vec::new();
        let mut last_block = None;
        for &i in &self.macro_lines {
            if let Some(block) = self.math_block_at(i) {
                // A block is read once, however many of its lines define.
                if last_block != Some(block.first_line) {
                    last_block = Some(block.first_line);
                    sources.push(self.math_body(text, block));
                }
                continue;
            }
            let line = &text[self.line_start_byte(i)..self.line_end_byte(i)];
            scan_inline(line, 0, line.len(), |found| {
                sources.extend(span_body(&line[found.span]));
            });
        }
        let made = Macros::collect(sources);
        let changed = made.stamp() != self.macros.stamp();
        if changed {
            self.macros = Arc::new(made);
        }
        changed
    }
}
