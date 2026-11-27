use std::collections::HashSet;

use super::check::{Verdict, no_overlap};
use super::format::{self, Scenario, Step};
use super::session::Session;
use super::{CURRENT_STAGE, index, report, run};
use crate::text_editor::{PaintedFrame, PaintedLine};

#[test]
fn acceptance_catalog() {
    let scenarios = format::load_all().unwrap();
    assert!(!scenarios.is_empty(), "no scenario files");
    // A debugging aid: only the scenarios whose id contains this.
    let only = std::env::var("JADE_ACCEPTANCE").unwrap_or_default();
    let mut broken = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for scenario in scenarios.iter().filter(|s| s.id.contains(&only)) {
        let outcome = run(scenario);
        let text = report(scenario, &outcome);
        eprintln!("{text}");
        *counts.entry(outcome.status()).or_insert(0) += 1;
        // A mandatory scenario that could check nothing has not passed.
        let verified = matches!(outcome.status(), "pass" | "part");
        if scenario.from_stage <= CURRENT_STAGE && !verified {
            broken.push(text);
        }
    }
    eprintln!("acceptance, stage {CURRENT_STAGE}: {counts:?}");
    assert!(
        broken.is_empty(),
        "mandatory scenarios did not pass:\n{}",
        broken.join("\n")
    );
}

#[test]
fn acceptance_index_matches_the_scenario_files() {
    let wanted = index::render(&format::load_all().unwrap());
    let path = format::root().join("INDEX.md");
    if std::env::var_os("JADE_UPDATE_INDEX").is_some() {
        std::fs::write(&path, &wanted).unwrap();
    }
    let written = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        written == wanted,
        "INDEX.md is stale; rerun with JADE_UPDATE_INDEX=1"
    );
}

#[test]
fn acceptance_scenarios_cite_rules_that_exist() {
    let spec = include_str!("../../../../../SPEC.md");
    let rules: HashSet<&str> = spec
        .lines()
        .filter_map(|line| line.strip_prefix("| ")?.split(" |").next())
        .collect();
    for scenario in format::load_all().unwrap() {
        for rule in &scenario.spec {
            assert!(
                rules.contains(rule.as_str()),
                "{}: SPEC.md has no rule {rule}",
                scenario.id
            );
        }
    }
}

fn scenario(steps_and_expects: &str) -> Scenario {
    let head = "id = \"probe\"\nspec = []\nsource = \"test\"\nfrom_stage = 2\n\
                note = \"alpha\\n\\nbeta gamma\\n\\ndelta\\n\"\n";
    format::parse(&format!("{head}{steps_and_expects}")).unwrap()
}

#[test]
fn acceptance_runner_resizes_the_window() {
    use jade_debug_protocol::{Answer, Command, Query};
    let probe = scenario("[[expect]]\nno_overlap = true");
    let mut session = Session::open(&probe).unwrap();
    let size = |session: &mut Session| {
        session.command(Command::Query {
            what: Query::WindowSize,
        })
    };
    let (width, height) = (1100.0, 700.0);
    assert_eq!(size(&mut session), Answer::WindowSize { width, height });
    assert!(session.run_step(&Step::Resize("800x600".into())).is_ok());
    let (width, height) = (800.0, 600.0);
    assert_eq!(size(&mut session), Answer::WindowSize { width, height });
}

#[test]
fn acceptance_runner_composes_and_drags() {
    let outcome = run(&scenario(
        r#"
        [[step]]
        click = { line = 0, col = 0 }
        [[step]]
        compose = { preview = "´", commit = "é" }
        [[step]]
        drag = { from = { line = 2, col = 0 }, dx = 400.0 }
        [[step]]
        type = "dragged over"
        [[expect]]
        text = "éalpha\n\ndragged over\n\ndelta\n"
        [[expect]]
        caret_visible = true
        "#,
    ));
    assert_eq!(outcome.failures, Vec::<String>::new());
    assert_eq!(outcome.status(), "pass");
}

#[test]
fn acceptance_runner_measures_how_far_content_moved() {
    let long = |expects: &str| {
        let mut probe = scenario(&format!("[[step]]\nscroll = {{ dy = -200.0 }}\n{expects}"));
        probe.note = "a line\n".repeat(200);
        run(&probe)
    };
    let right = long("[[expect]]\ncontent_moved = -200.0");
    assert_eq!(right.status(), "pass", "{right:?}");
    let wrong = long("[[expect]]\ncontent_moved = -150.0\n[[expect]]\ncaret_visible = true");
    assert_eq!((wrong.passed, wrong.failures.len()), (0, 2), "{wrong:?}");
}

#[test]
fn acceptance_runner_checks_rows_and_the_caret_direction() {
    let long = "word ".repeat(60);
    let mut probe = scenario(
        r#"
        [[step]]
        click = { line = 2, col = 1 }
        [[step]]
        type = "x"
        [[expect]]
        caret_moved_right = true
        [[expect]]
        same_row = { line = 0, from = 0, to = 10 }
        [[expect]]
        other_row = { line = 0, from = 0, to = 290 }
        [[expect]]
        shown = [0, 2]
        [[expect]]
        height_at_most = { line = 2, other = 0, times = 1.0 }
        [[expect]]
        every_frame_consistent = true
        "#,
    );
    probe.note = format!("{long}\n\nshort\n");
    let outcome = run(&probe);
    assert_eq!(outcome.failures, Vec::<String>::new());
    assert_eq!(outcome.passed, 6);

    let mut wrong = scenario(
        r#"
        [[step]]
        click = { line = 2, col = 4 }
        [[step]]
        key = "left"
        [[expect]]
        caret_moved_right = true
        [[expect]]
        same_row = { line = 0, from = 0, to = 290 }
        [[expect]]
        other_row = { line = 0, from = 0, to = 10 }
        [[expect]]
        hidden = [2]
        [[expect]]
        shown = [99]
        [[expect]]
        height_at_most = { line = 0, other = 2, times = 1.0 }
        "#,
    );
    wrong.note = format!("{long}\n\nshort\n");
    let outcome = run(&wrong);
    assert_eq!(
        (outcome.passed, outcome.failures.len()),
        (0, 6),
        "{outcome:?}"
    );
}

/// Without this the catalog could report passes while checking nothing.
#[test]
fn acceptance_runner_drives_the_real_editor() {
    let outcome = run(&scenario(
        r#"
        [[step]]
        click = { line = 2, col = 5 }
        [[step]]
        type = "!"
        [[step]]
        paste = " pasted"
        [[step]]
        undo = {}
        [[step]]
        redo = {}
        [[step]]
        resize = "800x600"
        [[step]]
        scroll = { dy = -40.0 }
        [[step]]
        key = "down"
        [[expect]]
        text = "alpha\n\nbeta ! pastedgamma\n\ndelta\n"
        [[expect]]
        view_stable = true
        [[expect]]
        line_stable = [0, 4]
        [[expect]]
        no_overlap = true
        [[expect]]
        pixels = { target = "editor", expect = "ink:0,0,400,60" }
        "#,
    ));
    assert_eq!(outcome.failures, Vec::<String>::new());
    assert_eq!((outcome.passed, outcome.status()), (5, "pass"));
}

#[test]
fn acceptance_runner_fails_wrong_expectations() {
    let outcome = run(&scenario(
        r#"
        [[step]]
        click = { line = 0, col = 0 }
        [[step]]
        type = "wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words wrapping words "
        [[expect]]
        caret = { line = 0, col = 0 }
        [[expect]]
        text_contains = "missing"
        [[expect]]
        line_stable = [2]
        [[expect]]
        pixels = { target = "editor", expect = "empty:0,0,400,60" }
        "#,
    ));
    assert_eq!(
        (outcome.passed, outcome.failures.len()),
        (0, 4),
        "{outcome:?}"
    );
}

#[test]
fn acceptance_runner_fails_a_view_that_scrolled() {
    let mut probe = scenario("[[step]]\nscroll = { dy = -200.0 }\n[[expect]]\nview_stable = true");
    probe.note = "a line\n".repeat(200);
    let outcome = run(&probe);
    assert_eq!(outcome.status(), "FAIL", "{outcome:?}");
    assert!(outcome.failures[0].starts_with("scrolled"), "{outcome:?}");
}

#[test]
fn acceptance_runner_fails_lines_that_overlap() {
    let line = |line, y| PaintedLine {
        line,
        y,
        height: 20.0,
        in_region: false,
    };
    let mut frame = PaintedFrame {
        lines: vec![line(0, 0.0), line(1, 20.0)],
        ..Default::default()
    };
    assert_eq!(no_overlap(&frame), Verdict::Passed);
    frame.lines[1].y = 10.0;
    assert!(matches!(no_overlap(&frame), Verdict::Failed(_)));
    frame.lines.clear();
    assert!(matches!(no_overlap(&frame), Verdict::Skipped(_)));
}

#[test]
fn acceptance_runner_skips_what_it_cannot_answer() {
    let outcome = run(&scenario(
        "[[expect]]\nblock = { line = 0, kind = \"paragraph\" }",
    ));
    assert_eq!(outcome.status(), "SKIP", "{outcome:?}");
    let outcome = run(&scenario(
        "[[step]]\ntoggle = \"show-structure\"\n[[expect]]\ntext_contains = \"alpha\"",
    ));
    assert_eq!(outcome.status(), "SKIP", "{outcome:?}");
}

#[test]
fn acceptance_files_with_typos_do_not_load() {
    let head = "id = \"a\"\nspec = []\nsource = \"s\"\nfrom_stage = 2\nnote = \"\"\n";
    let expect = "[[expect]]\nno_overlap = true\n";
    assert!(format::parse(&format!("{head}{expect}")).is_ok());
    for bad in [
        format!("{head}form_stage = 2\n{expect}"),
        head.to_owned(),
        format!("{head}size = \"big\"\n{expect}"),
        format!("{head}[[step]]\npress = \"a\"\n{expect}"),
        format!("{head}[[step]]\nclick = {{ line = 1 }}\n{expect}"),
    ] {
        assert!(format::parse(&bad).is_err(), "{bad}");
    }
}
