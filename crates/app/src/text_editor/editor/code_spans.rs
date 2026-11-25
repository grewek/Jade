//! Highlights the fenced code near the viewport and hands the spans to the
//! view, which the shaping reads. Colour only: no row count changes.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::App;

use super::super::TextSettings;
use super::super::doc::DocumentCache;
use super::super::element::{block_lines, fence_language};
use super::super::highlight::{Highlighted, Syntaxes};
use super::super::preview::LineRole;
use super::super::settings::SyntaxStore;
use super::Editor;

/// Lines highlighted before the first paint says what is on screen.
const FIRST_FRAME_LINES: usize = 100;

impl Editor {
    /// Sets the view's code spans for the blocks in or near the last painted
    /// range; none when `code_block.highlight` is off. Called before render.
    pub(super) fn sync_code_spans(&mut self, cx: &App) {
        let on = TextSettings::typography(cx).code_block.highlight;
        let store = on
            .then(|| cx.try_global::<SyntaxStore>().cloned())
            .flatten();
        let blocks = store.map_or_else(Vec::new, |store| {
            let doc = self.document_cache.borrow();
            let range = self
                .last_view
                .as_ref()
                .map_or(0..FIRST_FRAME_LINES.min(doc.line_count()), |v| {
                    v.visible_lines()
                });
            code_blocks(&doc, self.buffer.text(), range, &mut store.0.borrow_mut())
        });
        self.document_cache.borrow_mut().view.set_code(blocks);
    }
}

/// The highlighted blocks that touch `range`, as `(content lines, spans)`.
fn code_blocks(
    doc: &DocumentCache,
    text: &str,
    range: Range<usize>,
    syntaxes: &mut Syntaxes,
) -> Vec<(Range<usize>, Arc<Highlighted>)> {
    let roles = doc.line_roles();
    let mut out = Vec::new();
    let mut i = range.start;
    while i < range.end.min(roles.len()) {
        if matches!(roles[i], LineRole::Plain) || doc.side_by_side(i).is_some() {
            i += 1;
            continue;
        }
        let block = block_lines(roles, |j| doc.fence_open_before(j), i);
        i = block.end.max(i + 1);
        if let Some(found) = block_spans(doc, text, &block, syntaxes) {
            out.push(found);
        }
    }
    out
}

fn block_spans(
    doc: &DocumentCache,
    text: &str,
    block: &Range<usize>,
    syntaxes: &mut Syntaxes,
) -> Option<(Range<usize>, Arc<Highlighted>)> {
    let roles = doc.line_roles();
    // The block runs from its opening fence; a closed one ends on the closing fence.
    if roles[block.start] != LineRole::FenceDelimiter {
        return None;
    }
    let language =
        fence_language(&text[doc.line_start_byte(block.start)..doc.line_end_byte(block.start)])?;
    let closed = block.end - 1 > block.start && roles[block.end - 1] == LineRole::FenceDelimiter;
    let content = block.start + 1..block.end - usize::from(closed);
    if content.is_empty() {
        return None;
    }
    let code = &text[doc.line_start_byte(content.start)..doc.line_end_byte(content.end - 1)];
    let spans = syntaxes.cache()?.spans(language, code)?;
    Some((content, spans))
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext, TestAppContext, WindowOptions};

    use super::super::super::TextSettings;
    use super::super::super::settings::SyntaxStore;
    use super::Editor;

    const TINY: &str = "%YAML 1.2\n---\nname: Tiny\nfile_extensions: [tiny]\nscope: source.tiny\n\
        contexts:\n  main:\n    - match: '\\b(?:let|fn)\\b'\n      scope: keyword.control.tiny\n";

    /// Lines of `text` that have spans after a sync, with highlighting `on`.
    fn coloured(cx: &mut TestAppContext, text: &str, on: bool) -> Vec<usize> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tiny.sublime-syntax"), TINY).unwrap();
        let folder = dir.path().to_path_buf();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::ThemeEngine::install(None, cx);
            SyntaxStore::set(Some(folder), cx);
            TextSettings::apply_typography(|t| t.code_block.highlight = on, cx);
        });
        let (_window, editor) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
                cx.new(|cx| Editor::new(text, cx))
            })
            .unwrap()
        });
        cx.update(|cx| editor.update(cx, |e, cx| e.sync_code_spans(cx)));
        cx.read(|cx| {
            let doc = editor.read(cx).document_cache.borrow();
            (0..doc.line_count())
                .filter(|&i| !doc.view.code_line(i).is_empty())
                .collect()
        })
    }

    const NOTE: &str =
        "let outside\n```tiny\nlet a\nplain\nfn b\n```\n```nope\nlet c\n```\n```tiny\nlet open\n";

    #[gpui_kit::test]
    fn only_content_lines_of_known_languages_get_spans(cx: &mut TestAppContext) {
        // 2 and 4 hold keywords; the unknown language, fences and prose none;
        // the unclosed block's last line is still its content.
        assert_eq!(coloured(cx, NOTE, true), [2, 4, 10]);
    }

    #[gpui_kit::test]
    fn the_switch_off_gives_no_spans(cx: &mut TestAppContext) {
        assert!(coloured(cx, NOTE, false).is_empty());
    }
}
