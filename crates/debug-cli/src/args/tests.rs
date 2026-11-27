use super::*;
use jade_debug_protocol::Check;

fn words(line: &str) -> Vec<String> {
    split_words(line).unwrap()
}

fn command(line: &str) -> Command {
    match parse_action(&words(line)).unwrap() {
        Action::Send(command) => command,
        _ => panic!("{line:?} is not a plain command"),
    }
}

#[test]
fn theme_queries_without_an_id_and_switches_with_one() {
    assert_eq!(command("theme"), Command::Query { what: Query::Theme });
    assert_eq!(
        command("theme mocha"),
        Command::SetTheme { id: "mocha".into() }
    );
    assert!(parse_action(&words("theme a b")).is_err());
}

#[test]
fn clicks_take_a_target_or_a_point_and_flags() {
    assert_eq!(
        command("click note-Big --double --shift"),
        Command::Click {
            at: Location::Target("note-Big".into()),
            button: Button::Left,
            count: 2,
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
        }
    );
    let Command::Click {
        at, button, count, ..
    } = command("click 10.5,20 --right")
    else {
        panic!("expected a click");
    };
    assert_eq!(at, Location::Point(Point { x: 10.5, y: 20.0 }));
    assert_eq!((button, count), (Button::Right, 1));
    assert!(parse_action(&words("click")).is_err());
    assert!(parse_action(&words("click a --bogus")).is_err());
}

#[test]
fn drag_takes_a_target_and_a_delta() {
    let Command::Drag {
        from,
        dx,
        dy,
        steps,
        ..
    } = command("drag sidebar-resize -40")
    else {
        panic!("expected a drag");
    };
    assert_eq!(from, Location::Target("sidebar-resize".into()));
    assert_eq!((dx, dy, steps), (-40.0, 0.0, DEFAULT_DRAG_STEPS));
    let Command::Drag { steps, .. } = command("drag a 10 5 3") else {
        panic!("expected a drag");
    };
    assert_eq!(steps, 3);
    assert!(parse_action(&words("drag a")).is_err());
    let Command::MouseMove { pressed, .. } = command("move 1,2 --held") else {
        panic!("expected a move");
    };
    assert_eq!(pressed, Some(Button::Left));
}

#[test]
fn scroll_requires_a_delta_and_defaults_dx() {
    let Command::Scroll { dx, dy, .. } = command("scroll editor-pane -300") else {
        panic!("expected a scroll");
    };
    assert_eq!((dx, dy), (0.0, -300.0));
    assert!(parse_action(&words("scroll editor-pane")).is_err());
    assert!(parse_action(&words("scroll editor-pane abc")).is_err());
}

#[test]
fn type_joins_words_and_unescapes() {
    assert_eq!(
        command("type hello   world"),
        Command::Type {
            text: "hello world".into()
        }
    );
    assert_eq!(
        command(r#"type "a\nb\tc\\d""#),
        Command::Type {
            text: "a\nb\tc\\d".into()
        }
    );
    assert_eq!(
        unescape(r"keep \q and trailing \"),
        r"keep \q and trailing \"
    );
    assert!(parse_action(&words("type")).is_err());
}

#[test]
fn key_chords_split_modifiers_from_the_key() {
    assert_eq!(
        parse_chord("enter").unwrap(),
        ("enter".into(), Modifiers::default())
    );
    let (key, m) = parse_chord("Ctrl+Shift+A").unwrap();
    assert_eq!(key, "a");
    assert!(m.ctrl && m.shift && !m.alt && !m.meta);
    assert_eq!(parse_chord("ctrl++").unwrap().0, "+");
    assert!(parse_chord("hyper+a").is_err());
    assert_eq!(
        parse_chord("+").unwrap(),
        ("+".into(), Modifiers::default())
    );
}

#[test]
fn observation_commands_and_defaults() {
    assert_eq!(
        command("text"),
        Command::ReadEditorText {
            start: 0,
            len: 2000
        }
    );
    assert_eq!(
        command("text 5 10"),
        Command::ReadEditorText { start: 5, len: 10 }
    );
    assert_eq!(
        command("await paint"),
        Command::Await {
            topic: Topic::Paint,
            timeout_ms: 5000
        }
    );
    assert!(parse_action(&words("await bogus")).is_err());
    assert!(matches!(
        parse_action(&words("frames --raw")).unwrap(),
        Action::Frames { raw: true }
    ));
    assert!(matches!(parse_action(&[]).unwrap(), Action::Help));
    assert!(parse_action(&words("frobnicate")).is_err());
}

#[test]
fn the_socket_comes_from_the_flag_then_the_environment_then_the_default() {
    let args = |s: &str| words(s);
    let cli = parse_cli(&args("--socket /a.sock targets"), Some("/env.sock".into())).unwrap();
    assert_eq!(cli.socket, PathBuf::from("/a.sock"));
    let cli = parse_cli(&args("targets"), Some("/env.sock".into())).unwrap();
    assert_eq!(cli.socket, PathBuf::from("/env.sock"));
    let cli = parse_cli(&args("targets"), None).unwrap();
    assert_eq!(cli.socket, PathBuf::from(DEFAULT_SOCKET));
    assert!(parse_cli(&args("--socket"), None).is_err());
}

#[test]
fn launch_parses_its_flags() {
    let Action::Launch(l) = parse_action(&words("launch /v --bin /x/app --in-place")).unwrap()
    else {
        panic!("expected launch");
    };
    assert_eq!(
        (l.vault, l.bin, l.in_place),
        ("/v".into(), Some("/x/app".into()), true)
    );
    assert!(parse_action(&words("launch")).is_err());
}

#[test]
fn quotes_group_words_and_unterminated_ones_are_errors() {
    assert_eq!(
        words(r#"type "two words" 'x y' z"#),
        ["type", "two words", "x y", "z"]
    );
    assert_eq!(words(r#"a "" b"#), ["a", "", "b"]);
    assert!(split_words(r#"type "oops"#).is_err());
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn facts_words_become_facts_with_their_options() {
    assert_eq!(
        command(
            "facts editor grid:32x8 rows cols bbox palette:4 pixel:3,4 --rect 1,2,30,40 --tol 5"
        ),
        Command::Facts {
            target: "editor".into(),
            facts: vec![
                Fact::InkGrid { cols: 32, rows: 8 },
                Fact::RowProfile,
                Fact::ColProfile,
                Fact::Bbox,
                Fact::Palette { max: 4 },
                Fact::Pixel { x: 3.0, y: 4.0 },
            ],
            rect: Some(rect(1.0, 2.0, 30.0, 40.0)),
            tolerance: 5,
        }
    );
    let Command::Facts {
        rect: None,
        tolerance,
        facts,
        ..
    } = command("facts editor palette")
    else {
        panic!("not facts");
    };
    assert_eq!(tolerance, DEFAULT_TOLERANCE);
    assert_eq!(facts, vec![Fact::Palette { max: 8 }]);
    for bad in [
        "facts",
        "facts editor",
        "facts editor grid:3",
        "facts editor wat",
        "facts editor bbox --rect 1,2",
    ] {
        assert!(parse_action(&words(bad)).is_err(), "{bad}");
    }
}

#[test]
fn expect_words_become_checks() {
    assert_eq!(
        command(
            "expect editor ink:1,2,3,4 empty:5,6,7,8,2% color:9,9,#1e1e2e,3 within:0,0,50,60 --tol 4"
        ),
        Command::Expect {
            target: "editor".into(),
            checks: vec![
                Check::InkIn {
                    rect: rect(1.0, 2.0, 3.0, 4.0),
                    min_percent: 0.1,
                },
                Check::Empty {
                    rect: rect(5.0, 6.0, 7.0, 8.0),
                    max_percent: 2.0,
                },
                Check::Color {
                    x: 9.0,
                    y: 9.0,
                    color: "#1e1e2e".into(),
                    tolerance: 3,
                },
                Check::BboxWithin {
                    rect: rect(0.0, 0.0, 50.0, 60.0),
                },
            ],
            tolerance: 4,
        }
    );
    for bad in [
        "expect",
        "expect editor",
        "expect editor ink:1,2,3",
        "expect editor nope:1",
        "expect editor color:1,2",
    ] {
        assert!(parse_action(&words(bad)).is_err(), "{bad}");
    }
}

#[test]
fn capture_takes_a_rect_a_scale_and_a_file() {
    let Action::Capture { command, out } = parse_action(&words(
        "capture editor --rect 0,0,100,50 --scale 0.5 --out /tmp/x.png",
    ))
    .unwrap() else {
        panic!("not a capture");
    };
    assert_eq!(
        command,
        Command::Capture {
            target: "editor".into(),
            rect: Some(rect(0.0, 0.0, 100.0, 50.0)),
            scale: Some(0.5),
        }
    );
    assert_eq!(out, PathBuf::from("/tmp/x.png"));
    let Action::Capture { out, .. } = parse_action(&words("capture note-Big")).unwrap() else {
        panic!("not a capture");
    };
    assert_eq!(out, PathBuf::from("/tmp/jade-capture-note-Big.png"));
    assert!(parse_action(&words("capture")).is_err());
    assert!(parse_action(&words("capture a b")).is_err());
    assert!(parse_action(&words("capture a --scale")).is_err());
}

#[test]
fn launch_can_ask_for_the_headless_mode() {
    let Action::Launch(args) = parse_action(&words("launch /v --headless --size 900x500")).unwrap()
    else {
        panic!("not a launch");
    };
    assert!(args.headless);
    assert_eq!(args.size.as_deref(), Some("900x500"));
    let Action::Launch(args) = parse_action(&words("launch /v")).unwrap() else {
        panic!("not a launch");
    };
    assert!(!args.headless && args.size.is_none());
}
