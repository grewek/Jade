//! What must hold for an open note after any input, for the debug socket's
//! invariants query (which the fuzzer asks after every step). On demand only:
//! it rebuilds the derived state of the whole note to compare.

use gpui_kit::{Context, Window};

use super::doc::DocumentCache;
use super::doc_stats::DocStats;
use super::element::render_params;
use super::link_table::LinkTable;
use super::rows::Inputs;
use super::{Editor, EditorBuffer};

/// Most lines whose row counts one check measures again (about 12 us each).
const MAX_CHECKED_ROWS: usize = 400;

/// What is wrong with where the caret and the selection are.
fn caret_problems(buffer: &EditorBuffer) -> Vec<String> {
    let text = buffer.text();
    let selection = buffer.selection();
    let places = [
        Some(("caret", buffer.cursor())),
        selection.as_ref().map(|s| ("selection start", s.start)),
        selection.as_ref().map(|s| ("selection end", s.end)),
    ];
    places
        .into_iter()
        .flatten()
        .filter(|&(_, at)| !text.is_char_boundary(at))
        .map(|(what, at)| {
            format!(
                "{what} at byte {at} is not a character boundary of the note ({} bytes)",
                text.len()
            )
        })
        .collect()
}

impl Editor {
    /// Everything wrong with the editor's state; empty when nothing is.
    pub(crate) fn state_problems(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let mut problems = caret_problems(&self.buffer);
        if !problems.is_empty() {
            // The checks below index the text with the caret.
            return problems;
        }
        let text = self.buffer.text();
        let doc = self.document_cache.borrow();
        let stale = doc.stale_fields(text);
        if !stale.is_empty() {
            problems.push(format!(
                "patched classification differs from a rebuild: {}",
                stale.join(", ")
            ));
        }
        // Built over a fresh cache, so a stale one cannot make these agree.
        let fresh = DocumentCache::new(text);
        if !self.doc_stats.same_counts(&DocStats::new(text)) {
            problems.push("patched word counts differ from a recount".into());
        }
        if !self.link_table.same_links(&LinkTable::new(text, &fresh)) {
            problems.push("the patched link table differs from a rescan".into());
        }
        if !stale.is_empty() {
            // Row counts follow the classification.
            return problems;
        }
        let params = render_params(self, window, cx);
        let rows = self.row_height_cache.borrow();
        let inp = Inputs {
            text,
            cursor: self.buffer.cursor(),
            focused: self.focus_handle.is_focused(window),
            width: rows.width(),
            font: &params.font,
            font_size: params.font_size,
            text_color: params.text_color,
        };
        let near = doc.line_at(self.buffer.cursor());
        for (line, counted, measured) in
            rows.inexact_rows(&doc, &inp, (near, MAX_CHECKED_ROWS), window)
        {
            problems.push(format!(
                "line {line} is counted as {counted} rows but measures {measured}"
            ));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, Window, WindowOptions};

    use super::Editor;
    use crate::text_editor::rows::correct_for_test;

    /// Every construct with patched state of its own.
    const NOTE: &str = "# Title\n\nwords and [[a link]] é\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n\
        > [!note]- Folded\n> body\n\n```\ncode\n```\n\n$$\nx^2\n$$\n";

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::ThemeEngine::install(None, cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Editor::new(NOTE, cx))
            })
            .expect("open test window")
        })
    }

    /// Draws two frames, so that the row counts are exact, and checks.
    fn problems(editor: &Entity<Editor>, window: &mut Window, cx: &mut gpui_kit::App) -> String {
        window.render_frame(cx);
        window.render_frame(cx);
        editor
            .update(cx, |editor, cx| editor.state_problems(window, cx))
            .join("; ")
    }

    #[gpui_kit::test]
    fn a_healthy_editor_has_no_problems_also_after_edits_and_a_fold(cx: &mut TestAppContext) {
        let (window, editor) = open(cx);
        cx.update_window(window, |_, window, cx| {
            assert_eq!(problems(&editor, window, cx), "");
            editor.update(cx, |editor, cx| editor.toggle_fold(0, cx));
            assert_eq!(problems(&editor, window, cx), "", "unfolded by the user");
            editor.update(cx, |editor, cx| {
                let end = editor.buffer.text().len();
                editor.buffer.set_cursor(end, false);
                for text in ["\n", "```", "\n[[b]] more"] {
                    let at = editor.buffer.cursor();
                    editor.buffer.insert(text);
                    editor.record_edit(
                        crate::text_editor::doc::TextEdit {
                            start_byte: at,
                            old_end_byte: at,
                            new_end_byte: at + text.len(),
                        },
                        window,
                        cx,
                    );
                }
            });
            assert_eq!(problems(&editor, window, cx), "");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn an_edit_the_derived_state_missed_is_reported(cx: &mut TestAppContext) {
        let (window, editor) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            editor.update(cx, |editor, cx| {
                editor.buffer.set_cursor(0, false);
                // Text changed behind the back of `record_edit`.
                editor.buffer.insert("[[new]] words\n```\n");
                let found = editor.state_problems(window, cx).join("; ");
                for part in [
                    "classification differs from a rebuild: line roles",
                    "word counts differ",
                    "link table differs",
                ] {
                    assert!(found.contains(part), "no {part:?} in {found}");
                }
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_wrong_row_count_is_reported(cx: &mut TestAppContext) {
        let (window, editor) = open(cx);
        cx.update_window(window, |_, window, cx| {
            assert_eq!(problems(&editor, window, cx), "");
            editor.update(cx, |editor, cx| {
                correct_for_test(&mut editor.row_height_cache.borrow_mut(), 2, 9);
                let found = editor.state_problems(window, cx);
                assert_eq!(found, ["line 2 is counted as 9 rows but measures 1"]);
            });
        })
        .unwrap();
    }
}
