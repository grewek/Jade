//! `query invariants`: the open note's state checks and the layout problems of
//! the frames since the last such query.

use gpui_kit::{App, WeakEntity};
use jade_debug_protocol::{Answer, MAX_PROBLEM_BYTES, MAX_PROBLEMS};

use super::commands::first_window;
use crate::JadeApp;
use crate::text_editor::take_problems;

pub(super) fn query(app: &WeakEntity<JadeApp>, cx: &mut App) -> Answer {
    let (Some(app), Some(handle)) = (app.upgrade(), first_window(cx)) else {
        return Answer::Error {
            message: "the app window is gone".into(),
        };
    };
    // Taken first: measuring below must not add to this answer.
    let mut problems: Vec<String> = take_problems()
        .into_iter()
        .map(|p| format!("frame: {p}"))
        .collect();
    let state = handle.update(cx, |_, window, cx| match app.read(cx).editor().cloned() {
        Some(editor) => editor.update(cx, |editor, cx| editor.state_problems(window, cx)),
        None => Vec::new(),
    });
    match state {
        Ok(state) => problems.splice(0..0, state),
        Err(err) => {
            return Answer::Error {
                message: format!("window unavailable: {err}"),
            };
        }
    };
    reply(problems)
}

/// The reply for `problems`, cut to what fits in one line of the protocol.
fn reply(mut problems: Vec<String>) -> Answer {
    let omitted = problems.len().saturating_sub(MAX_PROBLEMS);
    problems.truncate(MAX_PROBLEMS);
    for problem in &mut problems {
        problem.truncate(problem.floor_char_boundary(MAX_PROBLEM_BYTES));
    }
    Answer::Invariants { problems, omitted }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_debug_protocol::{MAX_LINE_BYTES, Message, encode_line};

    #[test]
    fn a_long_report_is_cut_to_fit_one_protocol_line() {
        // Control characters take the most room in JSON, six bytes each.
        let worst = "\u{1}".repeat(2 * MAX_PROBLEM_BYTES);
        let answer = reply(vec![worst; MAX_PROBLEMS + 5]);
        let Answer::Invariants { problems, omitted } = &answer else {
            panic!("expected invariants, got {answer:?}");
        };
        assert_eq!((problems.len(), *omitted), (MAX_PROBLEMS, 5));
        let line = encode_line(&Message::Response { req: 1, answer }).unwrap();
        assert!(line.len() <= MAX_LINE_BYTES, "{}", line.len());
    }

    #[test]
    fn cutting_a_problem_keeps_whole_characters() {
        let long = format!("a{}", "é".repeat(MAX_PROBLEM_BYTES));
        let Answer::Invariants { problems, .. } = reply(vec![long]) else {
            panic!("expected invariants");
        };
        assert_eq!(problems[0].len(), MAX_PROBLEM_BYTES - 1);
    }
}
