//! One run: a note and steps from a seed, until the app crashes or hangs.

use std::collections::VecDeque;
use std::path::Path;

use jade_debug_protocol::{
    Answer, AppState, Button, Command, Location, Modifiers, Point, Query, Target,
};

use super::actions::{self, Spot, Step};
use super::app::{App, Gone, NOTE};
use super::rng::Rng;

/// Steps shown with a fault.
const TRAIL: usize = 8;
/// What typing answers when no text field has the focus.
const NOT_CONSUMED: &str = "nothing consumed the typed text";
const DRAG_STEPS: u8 = 6;

/// A crash or hang, and where it happened.
#[derive(Debug)]
pub(super) struct Fault {
    pub gone: Gone,
    pub seed: u64,
    /// The step that ended the app; 0 is opening the note.
    pub step: u32,
    /// The last steps up to that one, oldest first.
    pub trail: Vec<String>,
}

/// Why a run stopped before its last step.
enum Stop {
    Gone(Gone),
    /// The fuzzer itself cannot go on.
    Broken(String),
}

impl From<Gone> for Stop {
    fn from(gone: Gone) -> Self {
        Stop::Gone(gone)
    }
}

struct Run {
    app: App,
    seed: u64,
    step: u32,
    trail: VecDeque<String>,
}

fn click(at: Location, count: u8, modifiers: Modifiers) -> Command {
    Command::Click {
        at,
        button: Button::Left,
        count,
        modifiers,
    }
}

fn key(chord: &str) -> Command {
    // `ctrl-shift-left`; the key itself may be `/` but never `-`.
    let mut parts = chord.split('-').rev();
    let key = parts.next().unwrap_or_default().to_owned();
    let mut modifiers = Modifiers::default();
    for part in parts {
        modifiers.ctrl |= part == "ctrl";
        modifiers.shift |= part == "shift";
    }
    Command::Key { key, modifiers }
}

fn only_enter_and_tab(text: &str) -> bool {
    text.chars().all(|c| c == '\n' || c == '\t')
}

/// The window point at `spot` of `pane`.
fn point_in(pane: &Target, spot: Spot) -> Point {
    Point {
        x: pane.x + pane.width * spot.x as f32 / 1000.,
        y: pane.y + pane.height * spot.y as f32 / 1000.,
    }
}

impl Run {
    /// Sends a command that has to succeed.
    fn send(&mut self, command: Command) -> Result<(), Stop> {
        match self.app.call(command.clone())? {
            Answer::Ok => Ok(()),
            other => Err(Stop::Broken(format!("{command:?} answered {other:?}"))),
        }
    }

    fn pane(&mut self) -> Result<Target, Stop> {
        let Answer::Targets { targets } = self.app.call(Command::ListTargets)? else {
            return Err(Stop::Broken("cannot list targets".into()));
        };
        targets
            .into_iter()
            .find(|t| t.name == "editor-pane" && t.visible)
            .ok_or_else(|| Stop::Broken("there is no editor pane".into()))
    }

    fn app_state(&mut self) -> Result<AppState, Stop> {
        match self.app.call(Command::Query {
            what: Query::AppState,
        })? {
            Answer::AppState(state) => Ok(state),
            other => Err(Stop::Broken(format!("no app state: {other:?}"))),
        }
    }

    /// Closes dialogs and reopens the note, should a step have left it.
    fn return_to_the_note(&mut self) -> Result<(), Stop> {
        let mut state = self.app_state()?;
        if state.dialog_open {
            self.send(key("escape"))?;
            state = self.app_state()?;
        }
        if state.note.as_deref() != Some(NOTE) || state.dialog_open {
            self.open_note()?;
        }
        Ok(())
    }

    fn open_note(&mut self) -> Result<(), Stop> {
        let note = Location::Target(format!("note-{NOTE}"));
        self.send(click(note, 1, Modifiers::default()))?;
        self.focus_editor()
    }

    fn focus_editor(&mut self) -> Result<(), Stop> {
        let editor = Location::Target("editor".into());
        self.send(click(editor, 1, Modifiers::default()))
    }

    /// Gives the editor the focus back if an earlier click took it, so that
    /// keys keep reaching it.
    fn keep_focus(&mut self) -> Result<(), Stop> {
        let query = Command::Query {
            what: Query::EditorState,
        };
        match self.app.call(query)? {
            Answer::EditorState(state) if state.focused => Ok(()),
            Answer::EditorState(_) => self.focus_editor(),
            other => Err(Stop::Broken(format!("no editor state: {other:?}"))),
        }
    }

    fn type_text(&mut self, text: &str) -> Result<(), Stop> {
        let command = Command::Type {
            text: text.to_owned(),
        };
        match self.app.call(command.clone())? {
            Answer::Ok => Ok(()),
            // The editor takes Enter and Tab without reporting them as used,
            // so a text of only these is answered as if nothing took it.
            Answer::Error { message }
                if message.starts_with(NOT_CONSUMED) && only_enter_and_tab(text) =>
            {
                Ok(())
            }
            other => Err(Stop::Broken(format!("{command:?} answered {other:?}"))),
        }
    }

    fn perform(&mut self, step: &Step) -> Result<(), Stop> {
        if matches!(step, Step::Type(_) | Step::Paste(_) | Step::Key(_)) {
            self.keep_focus()?;
        }
        match step {
            Step::Type(text) => self.type_text(text),
            Step::Paste(text) => {
                self.send(Command::SetClipboard { text: text.clone() })?;
                self.send(key("ctrl-v"))
            }
            Step::Key(chord) => self.send(key(chord)),
            Step::Click {
                at,
                count,
                modifiers,
            } => {
                let at = Location::Point(point_in(&self.pane()?, *at));
                self.send(click(at, *count, *modifiers))
            }
            Step::Drag { from, to } => {
                let pane = self.pane()?;
                let (from, to) = (point_in(&pane, *from), point_in(&pane, *to));
                self.send(Command::Drag {
                    from: Location::Point(from),
                    dx: to.x - from.x,
                    dy: to.y - from.y,
                    steps: DRAG_STEPS,
                    button: Button::Left,
                    modifiers: Modifiers::default(),
                })
            }
            Step::Scroll(dy) => self.send(Command::Scroll {
                at: Location::Target("editor-pane".into()),
                dx: 0.,
                dy: *dy as f32,
                modifiers: Modifiers::default(),
            }),
            Step::Resize(width, height) => self.send(Command::Resize {
                width: *width as f32,
                height: *height as f32,
            }),
        }
    }

    fn steps(&mut self, rng: &mut Rng, steps: u32, trace: bool) -> Result<(), Stop> {
        self.open_note()?;
        for number in 1..=steps {
            let step = actions::step(rng);
            self.step = number;
            if self.trail.len() == TRAIL {
                self.trail.pop_front();
            }
            self.trail.push_back(format!("{number}: {step}"));
            if trace {
                eprintln!("seed {} step {number}: {step}", self.seed);
            }
            self.perform(&step)?;
            self.return_to_the_note()?;
        }
        Ok(())
    }
}

/// Runs `steps` steps of `seed` on a new app. Returns the crash or hang that
/// ended it, if any; `Err` means the fuzzer itself failed.
pub(super) fn run(
    bin: &Path,
    (seed, steps): (u64, u32),
    trace: bool,
) -> Result<Option<Fault>, String> {
    let mut rng = Rng::new(seed);
    let note = actions::note(&mut rng);
    let mut run = Run {
        app: App::start(bin, &note, seed)?,
        seed,
        step: 0,
        trail: VecDeque::with_capacity(TRAIL),
    };
    let ended = run.steps(&mut rng, steps, trace);
    let Run {
        app,
        seed,
        step,
        trail,
    } = run;
    let gone = match ended {
        // A panic on the way out counts too.
        Ok(()) => app.quit().err(),
        Err(Stop::Broken(why)) => return Err(format!("seed {seed} step {step}: {why}")),
        Err(Stop::Gone(gone)) => Some(gone),
    };
    Ok(gone.map(|gone| Fault {
        gone,
        seed,
        step,
        trail: trail.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_become_keys_with_modifiers() {
        let expect = |chord: &str, name: &str, ctrl, shift| {
            let Command::Key { key, modifiers } = key(chord) else {
                panic!("not a key");
            };
            assert_eq!(
                (key.as_str(), modifiers.ctrl, modifiers.shift),
                (name, ctrl, shift)
            );
        };
        expect("left", "left", false, false);
        expect("ctrl-shift-left", "left", true, true);
        expect("ctrl-/", "/", true, false);
    }

    #[test]
    fn text_of_only_enter_and_tab_is_told_apart() {
        assert!(only_enter_and_tab("\n\t\n"));
        assert!(!only_enter_and_tab("\na"));
    }
}
