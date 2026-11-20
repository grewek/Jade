//! A headless `JadeApp` for benches (`benches/layout.rs`, `benches/
//! allocations.rs`): the same harness the app's own timing tests use, on
//! GPUI's test platform, so a frame runs the real layout and paint code
//! without a GPU or a visible window. Needs the `bench-support` feature.

use crate::theme::ThemeEngine;
use std::path::PathBuf;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Entity, Focusable, KeyBinding, TestAppContext, Window,
    WindowOptions, point, px,
};

pub use crate::test_alloc::{AllocSnapshot, CountingAllocator, reset, snapshot};

use crate::JadeApp;
use crate::text_editor::{BackTab, Editor, InsertTab};

/// The one note the vault holds.
const NOTE: &str = "Big";

pub struct HeadlessApp {
    cx: TestAppContext,
    window: AnyWindowHandle,
    app: Entity<JadeApp>,
    editor: Option<Entity<Editor>>,
    dir: PathBuf,
}

impl HeadlessApp {
    /// A window over a vault holding one note, `Big`, with `note_text`; the
    /// first frame is rendered, the note is not opened yet.
    pub fn new(note_text: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jade-bench-{}-{n}", std::process::id()));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            panic!("cannot create {dir:?}: {err}");
        }
        if let Err(err) = std::fs::write(dir.join(format!("{NOTE}.md")), note_text) {
            panic!("cannot write the note: {err}");
        }

        let cx = TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            ThemeEngine::install(None, cx);
        });
        let vault = dir.clone();
        let opened = cx.update(|cx| {
            cx.bind_keys(crate::text_editor::key_bindings());
            gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                cx.new(|cx| JadeApp::new(vault.clone(), window, cx))
            })
        });
        let Ok((window, app)) = opened else {
            panic!("cannot open the headless window");
        };
        let mut headless = Self {
            cx,
            window,
            app,
            editor: None,
            dir,
        };
        headless.frame();
        headless
    }

    fn with_window<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        match self
            .cx
            .update_window(self.window, |_, window, cx| f(window, cx))
        {
            Ok(result) => result,
            Err(err) => panic!("the headless window is gone: {err}"),
        }
    }

    /// Clicks the note (reading it, building the caches) and renders the
    /// first frame of it: the cold-open cost.
    pub fn open_note(&mut self) {
        self.with_window(|window, cx| {
            window.click(NOTE, cx);
            window.render_frame(cx);
        });
        let editor = self.app.read_with(&self.cx, |app, _| app.editor().cloned());
        let Some(editor) = editor else {
            panic!("clicking the note did not open an editor");
        };
        self.editor = Some(editor);
    }

    /// Focuses the open note's editor so keystrokes reach it.
    pub fn focus_editor(&mut self) {
        let Some(editor) = self.editor.clone() else {
            panic!("open_note first");
        };
        self.with_window(|window, cx| {
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
        });
    }

    /// Runs `op` inside one window update and returns the heap allocations
    /// it made (all zero unless the binary installs [`CountingAllocator`]).
    /// Everything that must be timed or counted together goes in one
    /// closure: a window update that leaves the window invalidated draws
    /// once more when it returns, outside the measured region.
    fn measured(&mut self, op: impl FnOnce(&mut Window, &mut App)) -> AllocSnapshot {
        self.with_window(|window, cx| {
            reset();
            op(window, cx);
            snapshot()
        })
    }

    /// One full frame: refresh, layout, paint (no GPU submit).
    pub fn frame(&mut self) -> AllocSnapshot {
        self.measured(|window, cx| window.render_frame(cx))
    }

    /// Dispatches one key through the real keystroke path, the edit only: a
    /// printable character, or a name like `backspace`.
    pub fn key(&mut self, key: &str, text: Option<&str>) -> AllocSnapshot {
        let keystroke = parse_key(key, text);
        self.measured(|window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        })
    }

    /// A key, then the frame it causes.
    pub fn key_and_frame(&mut self, key: &str, text: Option<&str>) -> AllocSnapshot {
        let keystroke = parse_key(key, text);
        self.measured(|window, cx| {
            window.dispatch_keystroke(keystroke, cx);
            window.render_frame(cx);
        })
    }

    /// Sets the left sidebar's width (what a drag step does), then renders the
    /// frame: the editor's wrap width changes, so its row cache rebuilds.
    pub fn resize_sidebar_and_frame(&mut self, width: f32) -> AllocSnapshot {
        let app = self.app.clone();
        self.measured(|window, cx| {
            app.update(cx, |app, cx| {
                app.sidebar_width = px(width);
                cx.notify();
            });
            window.render_frame(cx);
        })
    }

    /// Switches to the built-in theme `id`, then renders the frame: every
    /// color changes, no layout does.
    pub fn set_theme_and_frame(&mut self, id: &str) -> AllocSnapshot {
        self.measured(|window, cx| {
            if let Err(messages) = ThemeEngine::set(id, cx) {
                panic!("cannot switch to {id}: {messages:?}");
            }
            window.render_frame(cx);
        })
    }

    /// Turns justified text on or off (not saved), then renders the frame.
    pub fn set_justify_and_frame(&mut self, on: bool) -> AllocSnapshot {
        self.measured(|window, cx| {
            cx.set_global(crate::text_editor::TextSettings { justify: on });
            window.render_frame(cx);
        })
    }

    /// Scrolls the editor pane to `fraction` (0..=1) of its range, then
    /// renders the frame.
    pub fn scroll_and_frame(&mut self, fraction: f32) -> AllocSnapshot {
        let Some(editor) = self.editor.clone() else {
            panic!("open_note first");
        };
        self.measured(|window, cx| {
            let handle = editor.read(cx).scroll_handle().clone();
            let max_y = f32::from(handle.max_offset().y.abs());
            handle.set_offset(point(px(0.), px(-max_y * fraction)));
            window.render_frame(cx);
        })
    }
}

fn parse_key(key: &str, text: Option<&str>) -> gpui_kit::Keystroke {
    let Ok(mut keystroke) = gpui_kit::Keystroke::parse(key) else {
        panic!("bad key {key:?}");
    };
    keystroke.key_char = text.map(str::to_owned);
    keystroke
}

impl Drop for HeadlessApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
