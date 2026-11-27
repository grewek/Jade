//! Compares an expectation with what the running path can tell. An answer it
//! cannot give is [`Verdict::Skipped`], never a pass.

use jade_debug_protocol::{Answer, Check, Command, DEFAULT_TOLERANCE};

use super::format::{Expect, Span};
use super::session::{Session, StepError};
use crate::text_editor::{PaintedFrame, take_problems};

/// Pixel disagreement below this is rounding.
const TOLERANCE_PX: f32 = 0.5;
const SAME_ROW_PX: f32 = 8.0;

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Passed,
    Failed(String),
    Skipped(String),
}

/// What the view was before the last step.
pub(super) struct Before {
    pub scroll_y: f32,
    pub frame: PaintedFrame,
    pub frame_number: u64,
    /// The caret's window x, if its line was drawn.
    pub caret_x: Option<f32>,
}

fn verdict(ok: bool, why: impl FnOnce() -> String) -> Verdict {
    if ok {
        Verdict::Passed
    } else {
        Verdict::Failed(why())
    }
}

pub(super) fn check(expect: &Expect, before: &Before, session: &mut Session) -> Verdict {
    evaluate(expect, before, session).unwrap_or_else(Verdict::Failed)
}

fn evaluate(expect: &Expect, before: &Before, session: &mut Session) -> Result<Verdict, String> {
    Ok(match expect {
        Expect::Caret { line, col } => {
            let got = session.caret()?;
            verdict(got == (*line, *col), || {
                format!(
                    "caret at line {} col {}, expected line {line} col {col}",
                    got.0, got.1
                )
            })
        }
        Expect::Text(want) => {
            let got = session.text()?;
            verdict(&got == want, || {
                format!("text is {got:?}, expected {want:?}")
            })
        }
        Expect::TextContains(want) => {
            let got = session.text()?;
            verdict(got.contains(want.as_str()), || {
                format!("text {got:?} does not contain {want:?}")
            })
        }
        Expect::Block { line, kind } => Verdict::Skipped(format!(
            "block (line {line} is {kind:?}): this rendering path reports no block kinds"
        )),
        Expect::ViewStable(true) => {
            let (scroll, now) = session.view()?;
            view_stable((before.scroll_y, &before.frame), (scroll, &now))
        }
        Expect::LineStable(lines) => {
            let (_, now) = session.view()?;
            lines_stable(lines, &before.frame, &now)
        }
        Expect::ContentMoved(by) => content_moved(*by, &before.frame, &session.view()?.1),
        Expect::NoOverlap(true) => no_overlap(&session.view()?.1),
        Expect::CaretVisible(true) => caret_visible(session)?,
        Expect::CaretMovedRight(true) => {
            let (line, _) = session.caret()?;
            if session.in_table(line)? {
                Verdict::Skipped("caret_moved_right: no caret positions inside tables".into())
            } else {
                caret_moved_right(before.caret_x, session.caret_x()?)
            }
        }
        Expect::EveryFrameConsistent(true) => every_frame_consistent(),
        Expect::Hidden(lines) => hidden(lines, &session.view()?.1),
        Expect::Shown(lines) => shown(lines, &session.view()?.1),
        Expect::HeightAtMost { line, other, times } => {
            height_at_most((*line, *other), *times, &session.view()?.1)
        }
        Expect::SameRow(span) => rows(span, true, session)?,
        Expect::OtherRow(span) => rows(span, false, session)?,
        Expect::ViewStable(false)
        | Expect::NoOverlap(false)
        | Expect::CaretVisible(false)
        | Expect::CaretMovedRight(false)
        | Expect::EveryFrameConsistent(false) => {
            Verdict::Failed("only `true` is meaningful; drop the expectation".into())
        }
        Expect::MaxFrameMs(limit) => max_frame(*limit, before.frame_number),
        Expect::Pixels { target, expect } => pixels(target, expect, session)?,
    })
}

fn view_stable(before: (f32, &PaintedFrame), now: (f32, &PaintedFrame)) -> Verdict {
    if (before.0 - now.0).abs() > TOLERANCE_PX {
        return Verdict::Failed(format!("scrolled from {} to {}", before.0, now.0));
    }
    let Some(anchor) = before.1.lines.first() else {
        return Verdict::Skipped("view_stable: no line was traced".into());
    };
    lines_stable(&[anchor.line], before.1, now.1)
}

fn lines_stable(lines: &[usize], before: &PaintedFrame, now: &PaintedFrame) -> Verdict {
    for &line in lines {
        let (Some(was), Some(is)) = (before.y_of(line), now.y_of(line)) else {
            return Verdict::Failed(format!("line {line} was not painted both times"));
        };
        if (was - is).abs() > TOLERANCE_PX {
            return Verdict::Failed(format!("line {line} moved from y={was:.1} to y={is:.1}"));
        }
    }
    Verdict::Passed
}

pub(super) fn content_moved(by: f32, before: &PaintedFrame, now: &PaintedFrame) -> Verdict {
    let mut compared = 0;
    for was in before.lines.iter().filter(|l| !l.in_region) {
        let Some(y) = now.y_of(was.line) else {
            continue;
        };
        compared += 1;
        let moved = y - was.y;
        if (moved - by).abs() > TOLERANCE_PX {
            return Verdict::Failed(format!(
                "line {} moved by {moved:.1}, expected {by:.1}",
                was.line
            ));
        }
    }
    verdict(compared > 0, || "no line was drawn both times".into())
}

fn caret_visible(session: &mut Session) -> Result<Verdict, String> {
    let pane = session.target("editor-pane")?;
    let Some(caret) = session.caret_bounds()? else {
        return Ok(Verdict::Failed("the caret's line is not drawn".into()));
    };
    let inside = caret.top() >= pane.top() && caret.bottom() <= pane.bottom();
    Ok(verdict(inside, || {
        format!(
            "caret from y={:?} to y={:?}, pane from y={:?} to y={:?}",
            caret.top(),
            caret.bottom(),
            pane.top(),
            pane.bottom()
        )
    }))
}

fn caret_moved_right(was: Option<f32>, is: Option<f32>) -> Verdict {
    match (was, is) {
        (Some(was), Some(is)) => verdict(is > was + TOLERANCE_PX, || {
            format!("the caret went from x={was:.1} to x={is:.1}")
        }),
        _ => Verdict::Failed("the caret's line was not drawn both times".into()),
    }
}

fn every_frame_consistent() -> Verdict {
    let mut problems = take_problems();
    problems.dedup();
    verdict(problems.is_empty(), || problems.join("; "))
}

/// The drawn height of a line; zero for one the frame left out.
fn height_of(line: usize, frame: &PaintedFrame) -> f32 {
    frame
        .lines
        .iter()
        .find(|l| l.line == line)
        .map_or(0., |l| l.height)
}

pub(super) fn hidden(lines: &[usize], frame: &PaintedFrame) -> Verdict {
    if frame.lines.is_empty() {
        return Verdict::Skipped("hidden: no line was traced".into());
    }
    match lines.iter().find(|&&l| height_of(l, frame) > TOLERANCE_PX) {
        Some(&line) => Verdict::Failed(format!(
            "line {line} is drawn, {:.1} tall",
            height_of(line, frame)
        )),
        None => Verdict::Passed,
    }
}

pub(super) fn shown(lines: &[usize], frame: &PaintedFrame) -> Verdict {
    match lines.iter().find(|&&l| height_of(l, frame) <= TOLERANCE_PX) {
        Some(line) => Verdict::Failed(format!("line {line} is not drawn")),
        None => Verdict::Passed,
    }
}

pub(super) fn height_at_most(
    (line, other): (usize, usize),
    times: f32,
    frame: &PaintedFrame,
) -> Verdict {
    let (tall, base) = (height_of(line, frame), height_of(other, frame));
    if tall <= 0. || base <= 0. {
        return Verdict::Failed(format!("lines {line} and {other} are not both drawn"));
    }
    verdict(tall <= base * times + TOLERANCE_PX, || {
        format!("line {line} is {tall:.1} tall, over {times} times line {other}'s {base:.1}")
    })
}

fn rows(span: &Span, same: bool, session: &mut Session) -> Result<Verdict, String> {
    let mut points = Vec::with_capacity(2);
    for col in [span.from, span.to] {
        match session.point_of(span.line, col) {
            Ok(point) => points.push(point),
            Err(StepError::Unsupported(why)) => return Ok(Verdict::Skipped(why)),
            Err(StepError::Failed(why)) => return Err(why),
        }
    }
    let (from, to) = (points[0], points[1]);
    // Half a body row apart is another row; less is one row's own jitter, such
    // as a formula's baseline.
    let apart = (from.y - to.y).abs() > SAME_ROW_PX;
    Ok(verdict(apart != same, || {
        format!(
            "line {} cols {} and {} are drawn at y={:.1} and y={:.1}",
            span.line, span.from, span.to, from.y, to.y
        )
    }))
}

pub(super) fn no_overlap(frame: &PaintedFrame) -> Verdict {
    if frame.lines.is_empty() {
        return Verdict::Skipped("no_overlap: no line was traced".into());
    }
    if !frame.problems.is_empty() {
        return Verdict::Failed(frame.problems.join("; "));
    }
    // Lines inside side-by-side regions are checked by the trace itself.
    let mut top_level: Vec<_> = frame.lines.iter().filter(|l| !l.in_region).collect();
    top_level.sort_by(|a, b| a.y.total_cmp(&b.y));
    match top_level
        .windows(2)
        .find(|p| p[1].y + TOLERANCE_PX < p[0].y + p[0].height)
    {
        Some(pair) => Verdict::Failed(format!(
            "line {} (y={:.1}) starts inside line {} (y={:.1}, h={:.1})",
            pair[1].line, pair[1].y, pair[0].line, pair[0].y, pair[0].height
        )),
        None => Verdict::Passed,
    }
}

fn max_frame(limit_ms: f32, since: u64) -> Verdict {
    if cfg!(debug_assertions) {
        return Verdict::Skipped("max_frame_ms: only a `--release` run times frames".into());
    }
    let slowest = crate::debug_server::frames::recent()
        .iter()
        .filter(|f| f.frame > since)
        .map(|f| f.total_us())
        .max();
    match slowest {
        Some(us) => verdict(us as f32 / 1000. <= limit_ms, || {
            format!(
                "slowest frame {:.2} ms, limit {limit_ms} ms",
                us as f32 / 1000.
            )
        }),
        None => Verdict::Skipped("max_frame_ms: the last step drew no editor frame".into()),
    }
}

fn pixels(target: &str, checks: &str, session: &mut Session) -> Result<Verdict, String> {
    let checks = checks
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<Vec<Check>, _>>()?;
    let answer = session.command(Command::Expect {
        target: target.into(),
        checks,
        tolerance: DEFAULT_TOLERANCE,
    });
    match answer {
        Answer::Expect { passed, results } => Ok(verdict(passed, || {
            results
                .iter()
                .filter(|r| !r.passed)
                .map(|r| r.detail.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })),
        Answer::Error { message } => Err(message),
        other => Err(format!("unexpected answer {other:?}")),
    }
}
