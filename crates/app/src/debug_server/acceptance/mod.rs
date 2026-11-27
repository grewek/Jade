//! The acceptance catalog of the rendering core (#138). Every scenario file
//! under `crates/app/tests/acceptance/` runs against the current rendering
//! path:
//!
//! `cargo test -p jade-app acceptance -- --test-threads=1 --nocapture`
//!
//! It needs a GPU (the headless renderer). It lives here rather than in
//! `tests/` because it drives the test-only debug server.

mod check;
mod format;
mod index;
mod session;
mod tests;

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::text_editor::take_problems;
use check::{Before, Verdict};
use format::Scenario;
use session::{Session, StepError};

/// Scenarios up to this stage of #130 must pass in full; later ones run and
/// report.
/// Each stage's pull request raises it.
const CURRENT_STAGE: u8 = 1;

#[derive(Debug, Default)]
struct Outcome {
    failures: Vec<String>,
    skipped: Vec<String>,
    passed: usize,
    /// The note never opened, so nothing about the editor was looked at.
    unopened: bool,
}

impl Outcome {
    fn status(&self) -> &'static str {
        match (
            self.failures.is_empty(),
            self.passed,
            self.skipped.is_empty(),
        ) {
            (false, _, _) => "FAIL",
            (true, 0, _) => "SKIP",
            (true, _, false) => "part",
            (true, _, true) => "pass",
        }
    }
}

/// Whether this outcome fails the build at `stage`. From a scenario's stage on,
/// a skipped step or check counts against it: otherwise part of it could go
/// unchecked without anyone noticing. A scenario that did not open fails at
/// every stage: without a graphics adapter the whole catalog would report only
/// failures that are allowed for now, and pass.
fn breaks_the_build(from_stage: u8, stage: u8, outcome: &Outcome) -> bool {
    outcome.unopened || (from_stage <= stage && outcome.status() != "pass")
}

fn run(scenario: &Scenario) -> Outcome {
    // A scenario that panics the editor must not hide the ones after it.
    catch_unwind(AssertUnwindSafe(|| play(scenario))).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("no message");
        Outcome {
            failures: vec![format!("panicked: {message}")],
            ..Default::default()
        }
    })
}

fn play(scenario: &Scenario) -> Outcome {
    let mut outcome = Outcome::default();
    take_problems();
    let mut session = match Session::open(scenario) {
        Ok(session) => session,
        Err(err) => {
            outcome.failures.push(format!("cannot open: {err}"));
            outcome.unopened = true;
            return outcome;
        }
    };
    let mut before = match snapshot(&mut session) {
        Ok(before) => before,
        Err(err) => {
            outcome.failures.push(err);
            return outcome;
        }
    };
    for (number, step) in scenario.steps.iter().enumerate() {
        let result = snapshot(&mut session)
            .map_err(StepError::Failed)
            .and_then(|view| {
                before = view;
                // Only the last step's frames are checked.
                take_problems();
                session.run_step(step)
            });
        match result {
            Ok(()) => {}
            Err(StepError::Unsupported(why)) => {
                outcome.skipped.push(format!("step {}: {why}", number + 1));
                return outcome;
            }
            Err(StepError::Failed(why)) => {
                outcome
                    .failures
                    .push(format!("step {} {step:?}: {why}", number + 1));
                return outcome;
            }
        }
    }
    for expect in &scenario.expects {
        match check::check(expect, &before, &mut session) {
            Verdict::Passed => outcome.passed += 1,
            Verdict::Failed(why) => outcome.failures.push(why),
            Verdict::Skipped(why) => outcome.skipped.push(why),
        }
    }
    outcome
}

fn snapshot(session: &mut Session) -> Result<Before, String> {
    let (scroll_y, frame) = session.view()?;
    Ok(Before {
        scroll_y,
        frame,
        caret_x: session.caret_x()?,
        frame_number: crate::debug_server::frames::current(),
    })
}

/// The report lines of one scenario.
fn report(scenario: &Scenario, outcome: &Outcome) -> String {
    let mut text = format!(
        "{} stage {} {}/{}",
        outcome.status(),
        scenario.from_stage,
        scenario.area,
        scenario.id
    );
    if !outcome.failures.is_empty() {
        text += &format!(
            "\n  spec {} · source {}",
            scenario.spec.join(", "),
            scenario.source
        );
    }
    for failure in &outcome.failures {
        text += &format!("\n  failed: {failure}");
    }
    for skipped in &outcome.skipped {
        text += &format!("\n  skipped: {skipped}");
    }
    text
}
