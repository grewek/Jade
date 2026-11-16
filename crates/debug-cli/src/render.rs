//! Human-readable output for answers (JSON for the rest, via the protocol's
//! own encoder so there is one definition of the wire format).

use jade_debug_protocol::{
    Answer, Event, FRAME_BUDGET_US, FrameSample, Message, Target, encode_line,
};

/// An encoded protocol line without its newline.
fn trimmed(encoded: Result<String, impl std::fmt::Display>) -> String {
    encoded.map_or_else(
        |err| format!("<encode error: {err}>"),
        |line| line.trim_end().to_owned(),
    )
}

pub(crate) fn event_line(event: &Event) -> String {
    trimmed(encode_line(&Message::Event {
        event: event.clone(),
    }))
}

/// What to print for `answer`; `Err` carries an app-side error message.
pub(crate) fn answer_text(answer: &Answer) -> Result<String, String> {
    Ok(match answer {
        Answer::Ok => "ok".into(),
        Answer::Error { message } => return Err(message.clone()),
        Answer::Targets { targets } => targets_table(targets),
        Answer::FrameTimes { frames } => frame_summary(frames),
        Answer::Text {
            text,
            start,
            total_len,
        } => {
            let end = start + text.len();
            let more = if end < *total_len {
                format!("  [{end}/{total_len} bytes, more follows]")
            } else {
                String::new()
            };
            format!("{text}{more}")
        }
        other => trimmed(encode_line(other)),
    })
}

fn targets_table(targets: &[Target]) -> String {
    if targets.is_empty() {
        return "no named elements (is the window visible and painting?)".into();
    }
    let mut out = format!("{:<30} {:>8} {:>8} {:>8} {:>8}", "name", "x", "y", "w", "h");
    for t in targets {
        let hidden = if t.visible { "" } else { "  [not visible]" };
        out.push_str(&format!(
            "\n{:<30} {:>8.1} {:>8.1} {:>8.1} {:>8.1}{hidden}",
            t.name, t.x, t.y, t.width, t.height
        ));
    }
    out
}

/// Percentile of an ascending slice (nearest rank).
fn percentile(sorted: &[u32], p: usize) -> u32 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

pub(crate) fn frame_summary(frames: &[FrameSample]) -> String {
    let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
        return "no frames recorded (is the window visible and painting?)".into();
    };
    let mut totals: Vec<u32> = frames.iter().map(FrameSample::total_us).collect();
    totals.sort_unstable();
    let ms = |us: u32| f64::from(us) / 1000.0;
    let over = totals.iter().filter(|&&t| t > FRAME_BUDGET_US).count();
    format!(
        "{} frames ({}..{}), editor render+paint: p50 {:.2}ms  p95 {:.2}ms  max {:.2}ms; \
         over the {:.2}ms budget: {over}",
        frames.len(),
        first.frame,
        last.frame,
        ms(percentile(&totals, 50)),
        ms(percentile(&totals, 95)),
        ms(totals[totals.len() - 1]),
        ms(FRAME_BUDGET_US),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(frame: u64, render_us: u32, paint_us: u32) -> FrameSample {
        FrameSample {
            frame,
            render_us,
            paint_us,
            interval_us: 0,
        }
    }

    #[test]
    fn frame_summaries_report_percentiles_and_budget_overruns() {
        let mut frames: Vec<_> = (1..=99).map(|i| frame(i, 100, 900)).collect();
        frames.push(frame(100, 10_000, 20_000));
        let text = frame_summary(&frames);
        assert!(text.contains("100 frames (1..100)"), "{text}");
        assert!(text.contains("p50 1.00ms"), "{text}");
        assert!(text.contains("max 30.00ms"), "{text}");
        assert!(text.contains("budget: 1"), "{text}");
        assert!(frame_summary(&[]).contains("no frames"));
    }

    #[test]
    fn targets_render_as_a_table_and_flag_hidden_ones() {
        let target = |name: &str, visible| Target {
            name: name.into(),
            x: 1.0,
            y: 2.5,
            width: 30.0,
            height: 4.0,
            visible,
        };
        let text = targets_table(&[target("a", true), target("b", false)]);
        assert!(text.lines().next().unwrap().starts_with("name"));
        assert!(text.contains("a") && text.contains("2.5"));
        assert!(text.lines().last().unwrap().ends_with("[not visible]"));
        assert!(targets_table(&[]).contains("no named elements"));
    }

    #[test]
    fn errors_are_errors_and_text_notes_when_more_follows() {
        assert_eq!(answer_text(&Answer::Ok).unwrap(), "ok");
        assert_eq!(
            answer_text(&Answer::Error {
                message: "boom".into()
            })
            .unwrap_err(),
            "boom"
        );
        let partial = Answer::Text {
            start: 0,
            text: "abc".into(),
            total_len: 10,
        };
        assert_eq!(
            answer_text(&partial).unwrap(),
            "abc  [3/10 bytes, more follows]"
        );
        let whole = Answer::Text {
            start: 0,
            text: "abc".into(),
            total_len: 3,
        };
        assert_eq!(answer_text(&whole).unwrap(), "abc");
        assert!(
            answer_text(&Answer::WindowSize {
                width: 1.0,
                height: 2.0
            })
            .unwrap()
            .contains("window_size")
        );
    }
}
