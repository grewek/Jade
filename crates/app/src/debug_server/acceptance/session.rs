//! One headless app with a scenario's note open: the real text system and the
//! headless renderer, so wrapping, click positions and pixels are the ones a
//! user gets. Steps go through the debug socket's own command handlers,
//! called directly on the main thread.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ClipboardItem, Entity, EntityInputHandler, Focusable,
    HeadlessAppContext, Keystroke, Pixels, WindowBounds, WindowOptions, point, px, size,
};
use jade_debug_protocol::{Answer, Button, Command, Location, Modifiers, Point};
use tempfile::TempDir;

use super::format::{ClickAt, Scenario, Step, parse_size};
use crate::JadeApp;
use crate::assets::JadeAssets;
use crate::debug_server::{capture, commands};
use crate::text_editor::{Editor, PaintedFrame, last_painted_frame};

const NOTE: &str = "Note";
/// Frames drawn after a step: an edit, its repaint and any deferred notify.
const FRAMES_PER_STEP: usize = 3;
/// Held-button moves of a drag step.
const DRAG_STEPS: u8 = 8;
/// Makes the paint trace and frame clock record; no socket is bound.
const ENV_VAR: &str = "JADE_DEBUG_SOCKET";

/// Why a step did not run.
pub(super) enum StepError {
    /// The running path has no such input.
    Unsupported(String),
    Failed(String),
}

pub(super) struct Session {
    // Field order is drop order: the app goes before its folders.
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    app: Entity<JadeApp>,
    _env: [EnvVar; 2],
    _vault: TempDir,
    _config: TempDir,
}

/// An environment variable set for a session and put back after it, so the
/// tests that follow do not inherit it.
struct EnvVar {
    name: &'static str,
    was: Option<OsString>,
}

impl EnvVar {
    fn set(name: &'static str, value: &Path) -> Self {
        let was = std::env::var_os(name);
        // SAFETY: tests run single-threaded (`--test-threads=1`).
        unsafe { std::env::set_var(name, value) };
        Self { name, was }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        // SAFETY: as in `set`.
        unsafe {
            match &self.was {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

impl Session {
    pub(super) fn open(scenario: &Scenario) -> Result<Self, String> {
        let io = |err: std::io::Error| err.to_string();
        let vault = tempfile::tempdir().map_err(io)?;
        let config = tempfile::tempdir().map_err(io)?;
        std::fs::write(vault.path().join(format!("{NOTE}.md")), &scenario.note).map_err(io)?;
        std::fs::write(config.path().join("config.toml"), &scenario.config).map_err(io)?;
        let env = [
            // Never the user's own config: its settings would change results.
            EnvVar::set(crate::theme::CONFIG_DIR_ENV, config.path()),
            EnvVar::set(ENV_VAR, &vault.path().join("unused.sock")),
        ];

        let text_system = gpui_kit::platform::current_platform(true).text_system();
        let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(JadeAssets), || {
            gpui_kit::platform::current_headless_renderer()
        });
        cx.allow_parking();
        cx.update(|cx| {
            crate::app::setup(cx);
            cx.set_reduce_motion(true);
        });
        let (w, h) = parse_size(&scenario.size)?;
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(w), px(h)),
            })),
            ..Default::default()
        };
        let path = vault.path().to_path_buf();
        let (window, app) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, move |window, cx| {
                    cx.new(|cx| JadeApp::new(path, window, cx))
                })
            })
            .map_err(|err| format!("cannot open the headless window: {err}"))?;
        let mut session = Self {
            cx,
            window,
            app,
            _env: env,
            _vault: vault,
            _config: config,
        };
        session.open_note(scenario.focus)?;
        Ok(session)
    }

    fn open_note(&mut self, focus: bool) -> Result<(), String> {
        self.draw();
        self.input(click(Location::Target(format!("note-{NOTE}"))))?;
        if focus {
            let editor = self.editor()?;
            self.in_window(|window, cx| editor.read(cx).focus_handle(cx).focus(window, cx))?;
            self.draw();
        }
        Ok(())
    }

    fn in_window<R>(
        &mut self,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> Result<R, String> {
        self.cx
            .update_window(self.window, |_, window, cx| f(window, cx))
            .map_err(|err| format!("window unavailable: {err}"))
    }

    /// Headless has no frame loop, so this is one.
    fn draw(&mut self) {
        for _ in 0..FRAMES_PER_STEP {
            self.cx.run_until_parked();
            let drawn = self.in_window(|window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx);
            });
            assert!(drawn.is_ok(), "the scenario window closed");
        }
        self.cx.run_until_parked();
    }

    pub(super) fn editor(&mut self) -> Result<Entity<Editor>, String> {
        let app = self.app.clone();
        self.cx
            .update(|cx| app.read(cx).editor().cloned())
            .ok_or_else(|| "no note is open".into())
    }

    /// Runs one debug command on the main thread.
    pub(super) fn command(&mut self, command: Command) -> Answer {
        let app = self.app.downgrade();
        self.cx.update(|cx| {
            if capture::is_pixel_command(&command) {
                capture::execute(&command, cx).0
            } else {
                commands::execute(command, &app, cx)
            }
        })
    }

    fn input(&mut self, command: Command) -> Result<(), String> {
        match self.command(command) {
            Answer::Ok => {
                self.draw();
                Ok(())
            }
            Answer::Error { message } => Err(message),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    pub(super) fn run_step(&mut self, step: &Step) -> Result<(), StepError> {
        let result = match step {
            Step::Click(at) => self.location(at).and_then(|at| self.input(click(at))),
            Step::Drag { from, dx, dy } => self.location(from).and_then(|from| {
                self.input(Command::Drag {
                    from,
                    dx: *dx,
                    dy: *dy,
                    steps: DRAG_STEPS,
                    button: Button::Left,
                    modifiers: Modifiers::default(),
                })
            }),
            Step::Compose { preview, commit } => self.compose(preview, commit),
            Step::Key(chord) => self.press(chord),
            Step::Type(text) => self.input(Command::Type { text: text.clone() }),
            Step::Paste(text) => {
                let item = ClipboardItem::new_string(text.clone());
                self.cx.update(|cx| cx.write_to_clipboard(item));
                self.press("ctrl-v")
            }
            Step::Undo {} => self.press("ctrl-z"),
            Step::Redo {} => self.press("ctrl-shift-z"),
            Step::Scroll { dy } => self.input(Command::Scroll {
                at: Location::Target("editor-pane".into()),
                dx: 0.0,
                dy: *dy,
                modifiers: Modifiers::default(),
            }),
            Step::Resize(text) => self.resize(text),
            Step::Toggle(name) => {
                return Err(StepError::Unsupported(format!("no toggle {name:?} yet")));
            }
        };
        result.map_err(StepError::Failed)
    }

    fn press(&mut self, chord: &str) -> Result<(), String> {
        let stroke = Keystroke::parse(chord).map_err(|err| format!("{chord:?}: {err}"))?;
        let m = stroke.modifiers;
        self.input(Command::Key {
            key: stroke.key,
            modifiers: Modifiers {
                ctrl: m.control,
                alt: m.alt,
                shift: m.shift,
                meta: m.platform,
            },
        })
    }

    fn resize(&mut self, text: &str) -> Result<(), String> {
        let (w, h) = parse_size(text)?;
        self.in_window(|window, cx| {
            window.resize(size(px(w), px(h)));
            // The test platform's resize fires no callback of its own.
            window.bounds_changed(cx);
        })?;
        self.draw();
        Ok(())
    }

    fn location(&mut self, at: &ClickAt) -> Result<Location, String> {
        Ok(match at {
            ClickAt::Target { target } => Location::Target(target.clone()),
            ClickAt::Text { line, col } => Location::Point(self.point_of(*line, *col)?),
        })
    }

    /// The two calls a platform makes for a composition, with a frame between.
    fn compose(&mut self, preview: &str, commit: &str) -> Result<(), String> {
        let editor = self.editor()?;
        self.in_window(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, preview, None, window, cx);
            })
        })?;
        self.draw();
        self.in_window(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(None, commit, window, cx);
            })
        })?;
        self.draw();
        Ok(())
    }

    /// The window bounds of a named element in the last frame.
    pub(super) fn target(&mut self, name: &str) -> Result<Bounds<Pixels>, String> {
        let Answer::Targets { targets } = self.command(Command::ListTargets) else {
            return Err("cannot list targets".into());
        };
        let target = targets
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| format!("{name:?} was not painted"))?;
        Ok(Bounds {
            origin: point(px(target.x), px(target.y)),
            size: size(px(target.width), px(target.height)),
        })
    }

    /// Where the caret would be drawn at a UTF-16 offset, from the input
    /// method's caret rectangle; `None` when its line is not drawn.
    fn bounds_at(
        &mut self,
        offset: impl FnOnce(&str) -> Option<usize>,
    ) -> Result<Option<Bounds<Pixels>>, String> {
        let editor = self.editor()?;
        let bounds = self.target("editor")?;
        self.in_window(|window, cx| {
            editor.update(cx, |editor, cx| {
                let utf16 = offset(editor.buffer().text())?;
                editor.bounds_for_range(utf16..utf16, bounds, window, cx)
            })
        })
    }

    pub(super) fn caret_bounds(&mut self) -> Result<Option<Bounds<Pixels>>, String> {
        let (line, col) = self.caret()?;
        self.bounds_at(|text| utf16_offset(text, line, col))
    }

    /// The window point where the caret would be drawn at `line`/`col`, without
    /// moving the caret.
    fn point_of(&mut self, line: usize, col: usize) -> Result<Point, String> {
        let caret = self.bounds_at(|text| utf16_offset(text, line, col))?;
        let caret = caret.ok_or(format!(
            "line {line} col {col} does not exist or is not on screen"
        ))?;
        let middle = caret.origin.y + caret.size.height / 2.;
        Ok(Point {
            // Just inside the character, so the click is not on its boundary.
            x: f32::from(caret.origin.x) + 1.,
            y: middle.into(),
        })
    }

    pub(super) fn text(&mut self) -> Result<String, String> {
        let editor = self.editor()?;
        Ok(self
            .cx
            .update(|cx| editor.read(cx).buffer().text().to_owned()))
    }

    /// The caret as a zero-based line and column in characters.
    pub(super) fn caret(&mut self) -> Result<(usize, usize), String> {
        let Answer::EditorState(state) = self.command(Command::Query {
            what: jade_debug_protocol::Query::EditorState,
        }) else {
            return Err("cannot read the editor state".into());
        };
        let text = self.text()?;
        let before = text
            .get(..state.cursor)
            .ok_or("the caret is outside the text")?;
        let line_start = before.rfind('\n').map_or(0, |at| at + 1);
        Ok((
            before.matches('\n').count(),
            before[line_start..].chars().count(),
        ))
    }

    pub(super) fn view(&mut self) -> Result<(f32, PaintedFrame), String> {
        let editor = self.editor()?;
        let scroll = self
            .cx
            .update(|cx| f32::from(editor.read(cx).scroll_handle().offset().y));
        Ok((scroll, last_painted_frame()))
    }
}

fn click(at: Location) -> Command {
    Command::Click {
        at,
        button: Button::Left,
        count: 1,
        modifiers: Modifiers::default(),
    }
}

/// The UTF-16 offset of a line and character column, if the text has them.
fn utf16_offset(text: &str, line: usize, col: usize) -> Option<usize> {
    let mut lines = text.split('\n');
    let before: usize = lines
        .by_ref()
        .take(line)
        .map(|l| l.encode_utf16().count() + 1)
        .sum();
    let found = lines.next()?;
    let in_line = found.chars().take(col).map(char::len_utf16).sum::<usize>();
    (found.chars().count() >= col).then_some(before + in_line)
}
