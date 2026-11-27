//! One run: a note and steps from a seed, each step followed by the checks.

use std::collections::VecDeque;
use std::path::Path;

use jade_debug_protocol::{
    Answer, AppState, Button, Command, FRAME_BUDGET_US, FrameSample, Location, Modifiers, Point,
    Query, Target,
};

use super::actions::{self, Spot, Step};
use super::app::{App, Gone, NOTE};
use super::findings::{Findings, Kind, Seen};
use super::rng::Rng;

/// Steps shown with a finding.
const TRAIL: usize = 8;
/// What the first frame after opening a note or a resize may take
/// (`SPEC.md`'s two exceptions to the frame budget).
const FIRST_FRAME_BUDGET_US: u32 = 33_000;
/// What typing answers when no text field has the focus.
const NOT_CONSUMED: &str = "nothing consumed the typed text";
const DRAG_STEPS: u8 = 6;

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

struct Run<'a> {
    app: App,
    seed: u64,
    step: u32,
    trail: VecDeque<String>,
    last_frame: u64,
    /// The next frame is the first after opening or resizing.
    first_frame_next: bool,
    findings: &'a mut Findings,
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

/// The frames newer than `last` that are over budget, the first of them judged
/// by the larger budget when it is `first_after_a_change`.
fn slow_frames(frames: &[FrameSample], last: u64, first_after_a_change: bool) -> Vec<FrameSample> {
    frames
        .iter()
        .filter(|f| f.frame > last)
        .enumerate()
        .filter(|&(i, f)| {
            let budget = if i == 0 && first_after_a_change {
                FIRST_FRAME_BUDGET_US
            } else {
                FRAME_BUDGET_US
            };
            f.total_us() > budget
        })
        .map(|(_, f)| *f)
        .collect()
}

impl Run<'_> {
    fn record(&mut self, kind: Kind, text: &str) {
        let (seed, step, trail) = (self.seed, self.step, &self.trail);
        let new = self.findings.add(kind, text, || Seen {
            seed,
            step,
            trail: trail.iter().cloned().collect(),
        });
        if new {
            let line = text.lines().next().unwrap_or_default();
            eprintln!("seed {seed} step {step}: new finding: {line}");
        }
    }

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
        self.focus_editor()?;
        self.first_frame_next = true;
        Ok(())
    }

    fn editor_focused(&mut self) -> Result<bool, Stop> {
        let query = Command::Query {
            what: Query::EditorState,
        };
        match self.app.call(query)? {
            Answer::EditorState(state) => Ok(state.focused),
            other => Err(Stop::Broken(format!("no editor state: {other:?}"))),
        }
    }

    /// Checked, because `Key` succeeds even when nothing takes the key: steps
    /// without the focus would be lost unnoticed.
    fn focus_editor(&mut self) -> Result<(), Stop> {
        self.send(Command::FocusEditor)?;
        if self.editor_focused()? {
            Ok(())
        } else {
            Err(Stop::Broken("the editor did not take the focus".into()))
        }
    }

    /// Gives the editor the focus back if an earlier click took it, so that
    /// keys keep reaching it.
    fn keep_focus(&mut self) -> Result<(), Stop> {
        if self.editor_focused()? {
            Ok(())
        } else {
            self.focus_editor()
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
            Step::Resize(width, height) => {
                self.first_frame_next = true;
                self.send(Command::Resize {
                    width: *width as f32,
                    height: *height as f32,
                })
            }
        }
    }

    fn check(&mut self) -> Result<(), Stop> {
        // Frames first: the next query draws frames of its own.
        let times = Command::Query {
            what: Query::FrameTimes,
        };
        let Answer::FrameTimes { frames } = self.app.call(times)? else {
            return Err(Stop::Broken("no frame times".into()));
        };
        let slow = slow_frames(&frames, self.last_frame, self.first_frame_next);
        if let Some(newest) = frames.last().filter(|f| f.frame > self.last_frame) {
            self.last_frame = newest.frame;
            self.first_frame_next = false;
        }
        if let Some(worst) = slow.iter().max_by_key(|f| f.total_us()) {
            let text = format!("a frame took {:.1} ms", f64::from(worst.total_us()) / 1000.);
            self.record(Kind::SlowFrame, &text);
        }
        let query = Command::Query {
            what: Query::Invariants,
        };
        match self.app.call(query)? {
            Answer::Invariants { problems, .. } => {
                for problem in &problems {
                    self.record(Kind::Invariant, problem);
                }
                Ok(())
            }
            other => Err(Stop::Broken(format!("no invariants: {other:?}"))),
        }
    }

    fn steps(&mut self, rng: &mut Rng, steps: u32, trace: bool) -> Result<(), Stop> {
        self.open_note()?;
        self.check()?;
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
            self.check()?;
        }
        Ok(())
    }
}

/// Runs `steps` steps of `seed` on a new app and adds what is wrong to
/// `findings`. A crash or hang ends the run. `Err` means the fuzzer itself
/// failed.
pub(super) fn run(
    bin: &Path,
    (seed, steps): (u64, u32),
    trace: bool,
    findings: &mut Findings,
) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let note = actions::note(&mut rng);
    let mut run = Run {
        app: App::start(bin, &note, seed)?,
        seed,
        step: 0,
        trail: VecDeque::with_capacity(TRAIL),
        last_frame: 0,
        first_frame_next: true,
        findings,
    };
    let gone = match run.steps(&mut rng, steps, trace) {
        Ok(()) => {
            let Run {
                app,
                seed,
                step,
                trail,
                findings,
                ..
            } = run;
            if let Err(gone) = app.quit() {
                let (kind, text) = describe(&gone);
                findings.add(kind, text, || Seen {
                    seed,
                    step,
                    trail: trail.into(),
                });
            }
            return Ok(());
        }
        Err(Stop::Broken(why)) => {
            return Err(format!("seed {seed} step {}: {why}", run.step));
        }
        Err(Stop::Gone(gone)) => gone,
    };
    let (kind, text) = describe(&gone);
    run.record(kind, text);
    Ok(())
}

fn describe(gone: &Gone) -> (Kind, &str) {
    match gone {
        Gone::Crash(text) => (Kind::Crash, text),
        Gone::Hang(text) => (Kind::Hang, text),
    }
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

    #[test]
    fn only_new_frames_are_judged_and_the_first_after_a_change_gets_more_time() {
        let frame = |frame, render_us| FrameSample {
            frame,
            render_us,
            ..Default::default()
        };
        let frames = [
            frame(1, 90_000),
            frame(2, 20_000),
            frame(3, 20_000),
            frame(4, 5_000),
        ];
        let numbers = |slow: Vec<FrameSample>| slow.iter().map(|f| f.frame).collect::<Vec<_>>();
        assert_eq!(numbers(slow_frames(&frames, 1, false)), [2, 3]);
        assert_eq!(numbers(slow_frames(&frames, 1, true)), [3]);
        assert_eq!(numbers(slow_frames(&frames, 0, true)), [1, 2, 3]);
    }
}
