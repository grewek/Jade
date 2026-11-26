//! Highlights the fenced code near the viewport without ever parsing on the
//! frame: each render hands the view the spans that are ready and starts
//! background parses for the blocks that are not, nearest first. A block shows
//! plain until its spans arrive, then repaints; colour only, so no row moves.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui_kit::{AppContext, Context};

use super::super::TextSettings;
use super::super::doc::DocumentCache;
use super::super::element::{block_lines, fence_language};
use super::super::highlight::{Cancel, Highlighted, Highlighter, Key, Lookup, Syntaxes};
use super::super::preview::LineRole;
use super::super::settings::SyntaxStore;
use super::Editor;

/// Lines looked at before the first paint says what is on screen.
const FIRST_FRAME_LINES: usize = 100;

/// A block with no spans yet, as the scan found it.
struct Miss {
    /// Lines between the block and the middle of the screen.
    distance: usize,
    key: Key,
    language: String,
    /// The block's content in the note text.
    code: Range<usize>,
}

struct Scan {
    ready: Vec<(Range<usize>, Arc<Highlighted>)>,
    misses: Vec<Miss>,
}

impl Editor {
    /// Sets the view's code spans from what is ready and starts the rest on
    /// background tasks; none when `code_block.highlight` is off. The blocks
    /// looked at are the last painted range plus as much again on each side,
    /// so a scroll finds them coloured. Called before every render.
    pub(super) fn sync_code_spans(&mut self, cx: &mut Context<Self>) {
        let on = TextSettings::typography(cx).code_block.highlight;
        let store = on
            .then(|| cx.try_global::<SyntaxStore>().cloned())
            .flatten();
        let Some(store) = store else {
            self.document_cache.borrow_mut().view.set_code(Vec::new());
            return;
        };
        let folder = store.0.borrow_mut().start_load();
        if let Some(folder) = folder {
            spawn_load(store.clone(), folder, cx);
        }
        let scan = {
            let doc = self.document_cache.borrow();
            let shown = self
                .last_view
                .as_ref()
                .map_or(0..FIRST_FRAME_LINES.min(doc.line_count()), |v| {
                    v.visible_lines()
                });
            scan_blocks(&doc, self.buffer.text(), &shown, &mut store.0.borrow_mut())
        };
        let wanted: HashSet<Key> = scan.misses.iter().map(|m| m.key).collect();
        store.0.borrow_mut().cancel_unwanted(&wanted);
        self.start_parses(&store, scan.misses, cx);
        self.document_cache.borrow_mut().view.set_code(scan.ready);
    }

    /// Starts the parses of `misses`, nearest first, as far as the job limit allows.
    fn start_parses(&self, store: &SyntaxStore, mut misses: Vec<Miss>, cx: &mut Context<Self>) {
        let Some(highlighter) = store.0.borrow().highlighter() else {
            return;
        };
        misses.sort_by_key(|m| m.distance);
        for miss in misses {
            let Some(token) = store.0.borrow_mut().begin(miss.key) else {
                continue;
            };
            let code = self.buffer.text()[miss.code].to_owned();
            spawn_parse(
                (store.clone(), Arc::clone(&highlighter)),
                (miss.key, token),
                (miss.language, code),
                cx,
            );
        }
    }
}

/// Reads the definitions off the main thread; every window repaints when
/// they are in, as blocks can then be parsed.
fn spawn_load(store: SyntaxStore, folder: std::path::PathBuf, cx: &mut Context<Editor>) {
    cx.spawn(async move |this, cx| {
        let loaded = cx
            .background_spawn(async move { Highlighter::load_dir(&folder) })
            .await;
        for notice in &loaded.notices {
            eprintln!("syntax: {notice}");
        }
        store.0.borrow_mut().finish_load(loaded.highlighter);
        this.update(cx, |_, cx| cx.refresh_windows()).ok();
    })
    .detach();
}

/// Parses one block off the main thread and keeps the spans in the shared
/// store, even if the editor is gone by then.
fn spawn_parse(
    (store, highlighter): (SyntaxStore, Arc<Highlighter>),
    (key, token): (Key, Cancel),
    (language, code): (String, String),
    cx: &mut Context<Editor>,
) {
    cx.spawn(async move |this, cx| {
        let cancelled = Arc::clone(&token);
        let word = language.clone();
        let spans = cx
            .background_spawn(async move {
                highlighter.highlight_until(&word, &code, &|| cancelled.load(Ordering::Relaxed))
            })
            .await;
        report_if_stopped(&store, &language, spans.as_ref());
        store.0.borrow_mut().finish(key, &token, spans);
        this.update(cx, |_, cx| cx.notify()).ok();
    })
    .detach();
}

/// A definition that refers to a file the folder lacks stops the parse at the
/// first line that needs it; say so once per language, not per block or frame.
fn report_if_stopped(store: &SyntaxStore, language: &str, spans: Option<&Highlighted>) {
    let Some(line) = spans.and_then(|s| s.stopped_at) else {
        return;
    };
    if store.0.borrow_mut().first_report(language) {
        eprintln!(
            "syntax: the {language} definition refers to a file that is not in the syntax \
             folder; a block stops being coloured at its line {} (the rest is plain)",
            line + 1
        );
    }
}

/// The blocks near `shown`: those with spans ready, and those without.
fn scan_blocks(
    doc: &DocumentCache,
    text: &str,
    shown: &Range<usize>,
    syntaxes: &mut Syntaxes,
) -> Scan {
    let roles = doc.line_roles();
    let span = shown.len();
    let near = shown.start.saturating_sub(span)..(shown.end + span).min(roles.len());
    let middle = shown.start + span / 2;
    let mut scan = Scan {
        ready: Vec::new(),
        misses: Vec::new(),
    };
    let mut i = near.start;
    while i < near.end {
        if matches!(roles[i], LineRole::Plain) {
            i += 1;
            continue;
        }
        let block = block_lines(roles, |j| doc.fence_open_before(j), i);
        i = block.end.max(i + 1);
        let Some((content, language, code)) = block_parts(doc, text, &block) else {
            continue;
        };
        match syntaxes.lookup(language, &text[code.clone()]) {
            Lookup::Plain => {}
            Lookup::Hit(spans) => scan.ready.push((content, spans)),
            Lookup::Miss(key) => {
                // Until the new spans arrive an edited block keeps the old
                // ones, where its text did not change, instead of going plain.
                if let Some(before) = doc.view.code_block_at(content.start) {
                    let carried = before.carried_over(&text[code.clone()]);
                    scan.ready.push((content.clone(), Arc::new(carried)));
                }
                scan.misses.push(Miss {
                    distance: content.start.abs_diff(middle),
                    key,
                    language: language.to_owned(),
                    code,
                });
            }
        }
    }
    scan
}

/// A fenced block's content lines, its language word and its content bytes.
fn block_parts<'a>(
    doc: &DocumentCache,
    text: &'a str,
    block: &Range<usize>,
) -> Option<(Range<usize>, &'a str, Range<usize>)> {
    let roles = doc.line_roles();
    // The block runs from its opening fence; a closed one ends on the closing fence.
    if roles[block.start] != LineRole::FenceDelimiter {
        return None;
    }
    let opening = doc.line_start_byte(block.start)..doc.line_end_byte(block.start);
    let language = fence_language(&text[opening])?;
    let closed = block.end - 1 > block.start && roles[block.end - 1] == LineRole::FenceDelimiter;
    let content = block.start + 1..block.end - usize::from(closed);
    if content.is_empty() {
        return None;
    }
    let code = doc.line_start_byte(content.start)..doc.line_end_byte(content.end - 1);
    Some((content, language, code))
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext, EntityInputHandler, TestAppContext, WindowOptions};

    use super::super::super::TextSettings;
    use super::super::super::settings::SyntaxStore;
    use super::Editor;

    const TINY: &str = "%YAML 1.2\n---\nname: Tiny\nfile_extensions: [tiny]\nscope: source.tiny\n\
        contexts:\n  main:\n    - match: '\\b(?:let|fn)\\b'\n      scope: keyword.control.tiny\n";

    const NOTE: &str =
        "let outside\n```tiny\nlet a\nplain\nfn b\n```\n```nope\nlet c\n```\n```tiny\nlet open\n";

    struct Env {
        cx: TestAppContext,
        window: gpui_kit::AnyWindowHandle,
        editor: gpui_kit::Entity<Editor>,
        _dir: tempfile::TempDir,
    }

    fn open(cx: &mut TestAppContext, text: &str, on: bool) -> Env {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tiny.sublime-syntax"), TINY).unwrap();
        let folder = dir.path().to_path_buf();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::ThemeEngine::install(None, cx);
            SyntaxStore::set(Some(folder), cx);
            TextSettings::apply_typography(|t| t.code_block.highlight = on, cx);
        });
        let (window, editor) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
                cx.new(|cx| Editor::new(text, cx))
            })
            .unwrap()
        });
        Env {
            cx: cx.clone(),
            window,
            editor,
            _dir: dir,
        }
    }

    impl Env {
        /// One render's sync, background tasks not run yet.
        fn sync_only(&mut self) {
            self.cx
                .update(|cx| self.editor.update(cx, |e, cx| e.sync_code_spans(cx)));
        }

        /// One render's sync, then every background task finished.
        fn sync(&mut self) {
            self.sync_only();
            self.cx.run_until_parked();
        }

        /// The blocks whose spans the shared store keeps.
        fn cached(&self) -> usize {
            self.cx
                .read(|cx| cx.global::<SyntaxStore>().0.borrow().cached())
        }

        /// Lines with spans, as the view holds them now.
        fn coloured(&self) -> Vec<usize> {
            self.cx.read(|cx| {
                let doc = self.editor.read(cx).document_cache.borrow();
                (0..doc.line_count())
                    .filter(|&i| !doc.view.code_line(i).is_empty())
                    .collect()
            })
        }

        /// Types `text` with the caret at byte `at`, the real typing path.
        fn type_at(&mut self, at: usize, text: &str) {
            let (editor, text) = (self.editor.clone(), text.to_owned());
            self.cx
                .update_window(self.window, |_, window, cx| {
                    editor.update(cx, |e, cx| {
                        e.buffer_mut().set_cursor(at, false);
                        e.replace_text_in_range(None, &text, window, cx);
                    });
                })
                .unwrap();
        }
    }

    #[gpui_kit::test]
    fn spans_arrive_after_the_background_work_and_not_before(cx: &mut TestAppContext) {
        let mut env = open(cx, NOTE, true);
        env.cx
            .update(|cx| env.editor.update(cx, |e, cx| e.sync_code_spans(cx)));
        assert!(env.coloured().is_empty(), "nothing is parsed on the frame");
        env.cx.run_until_parked();
        // The definitions are in; the next render starts the parses, the one
        // after finds them: 2 and 4 hold keywords, the unknown language, the
        // fences and the prose none, and the unclosed block's last line counts.
        env.sync();
        env.sync();
        assert_eq!(env.coloured(), [2, 4, 10]);
    }

    #[gpui_kit::test]
    fn the_switch_off_gives_no_spans_and_starts_no_work(cx: &mut TestAppContext) {
        let mut env = open(cx, NOTE, false);
        for _ in 0..3 {
            env.sync();
        }
        assert!(env.coloured().is_empty());
    }

    #[gpui_kit::test]
    fn an_edited_block_gets_the_spans_of_its_new_text(cx: &mut TestAppContext) {
        let mut env = open(cx, "```tiny\nlet x\n```\n", true);
        for _ in 0..3 {
            env.sync();
        }
        assert_eq!(env.coloured(), [1]);
        // "fn " in front moves the keyword; the old spans must not be used.
        env.type_at(8, "x ");
        env.cx.update(|cx| {
            env.editor.update(cx, |e, cx| e.sync_code_spans(cx));
        });
        assert_eq!(
            env.coloured(),
            [1],
            "the edited block keeps its old colours until the new ones arrive"
        );
        for _ in 0..3 {
            env.sync();
        }
        let ranges = env.cx.read(|cx| {
            let doc = env.editor.read(cx).document_cache.borrow();
            doc.view
                .code_line(1)
                .iter()
                .map(|s| s.range.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 2..5, "text is now `x let x`");
    }

    #[gpui_kit::test]
    fn a_parse_for_text_that_was_edited_meanwhile_is_cancelled(cx: &mut TestAppContext) {
        let mut env = open(cx, "```tiny\nlet x\n```\n", true);
        for _ in 0..3 {
            env.sync();
        }
        let settled = env.cached();
        // Two edits with no task run in between: the parse started for the
        // first has not run when the second makes it unwanted.
        env.type_at(8, "x ");
        env.sync_only();
        env.type_at(8, "y ");
        env.sync_only();
        env.cx.run_until_parked();
        assert_eq!(
            env.cached(),
            settled + 1,
            "the first edit's parse was cancelled, only the last was kept"
        );
        env.sync();
        assert_eq!(env.coloured(), [1]);
    }

    /// The lines with spans right after an edit's sync, before any task ran.
    fn coloured_while_pending(env: &mut Env, at: usize, typed: &str) -> Vec<usize> {
        env.type_at(at, typed);
        env.sync_only();
        env.coloured()
    }

    #[gpui_kit::test]
    fn an_edit_does_not_flash_the_block_plain(cx: &mut TestAppContext) {
        let mut env = open(cx, "```tiny\nlet a\nplain\nfn b\n```\n", true);
        for _ in 0..3 {
            env.sync();
        }
        assert_eq!(env.coloured(), [1, 3]);
        // Typing in the middle line stays coloured; a new line `x` in front
        // is plain and the two keyword lines move down with their colours.
        assert_eq!(coloured_while_pending(&mut env, 14, "z"), [1, 3]);
        assert_eq!(coloured_while_pending(&mut env, 8, "x\n"), [2, 4]);
        env.cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn the_case_of_the_fence_language_does_not_matter(cx: &mut TestAppContext) {
        let mut env = open(cx, "```TINY\nlet a\n```\n```Tiny\nfn b\n```\n", true);
        for _ in 0..3 {
            env.sync();
        }
        assert_eq!(env.coloured(), [1, 4]);
    }

    #[gpui_kit::test]
    fn an_unclosed_fence_over_the_limits_stays_plain_and_starts_nothing(cx: &mut TestAppContext) {
        let text = format!("```tiny\n{}", "let a\n".repeat(3_000));
        let mut env = open(cx, &text, true);
        for _ in 0..3 {
            env.sync();
        }
        assert!(env.coloured().is_empty());
        assert_eq!(env.cached(), 0);
    }

    #[gpui_kit::test]
    fn code_in_a_column_is_coloured_too(cx: &mut TestAppContext) {
        let note = "<!-- jade:columns -->\n\n```tiny\nlet a\n```\n\n<!-- jade:next -->\n\n```tiny\nfn b\n```\n\n<!-- jade:end -->\n";
        let mut env = open(cx, note, true);
        // The layout sets a width; without one every region is stacked.
        env.cx.read(|cx| {
            let editor = env.editor.read(cx);
            editor
                .document_cache
                .borrow_mut()
                .view
                .set_column_width(Some(900.));
        });
        for _ in 0..3 {
            env.sync();
        }
        let in_region = env.cx.read(|cx| {
            let doc = env.editor.read(cx).document_cache.borrow();
            doc.side_by_side(3).is_some() && doc.side_by_side(9).is_some()
        });
        assert!(in_region, "the test note must really be a columns region");
        assert_eq!(env.coloured(), [3, 9]);
    }
}
