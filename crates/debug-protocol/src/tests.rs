use super::*;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
    let line = encode_line(&value).unwrap();
    assert_eq!(line.matches('\n').count(), 1);
    assert_eq!(decode_line::<T>(&line).unwrap(), value);
}

fn round_trip_commands(commands: Vec<Command>) {
    for (req, command) in commands.into_iter().enumerate() {
        roundtrip(Request {
            req: req as u64,
            command,
        });
    }
}

#[test]
fn input_commands_round_trip() {
    let at = Point { x: 1.5, y: 2.0 };
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let none = Modifiers::default();
    let editor = || Location::Target("editor".into());
    round_trip_commands(vec![
        Command::Type {
            text: "hi\n".into(),
        },
        Command::Key {
            key: "enter".into(),
            modifiers: Modifiers { ctrl: true, ..none },
        },
        Command::MouseMove {
            at: Location::Point(at),
            pressed: Some(Button::Left),
            modifiers: none,
        },
        Command::Drag {
            from: Location::Target("sidebar-resize".into()),
            dx: -40.0,
            dy: 0.0,
            steps: 4,
            button: Button::Left,
            modifiers: none,
        },
        Command::MouseDown {
            at: editor(),
            button: Button::Right,
            modifiers: shift,
        },
        Command::MouseUp {
            at: Location::Point(at),
            button: Button::Middle,
            modifiers: none,
        },
        Command::Click {
            at: editor(),
            button: Button::Left,
            count: 2,
            modifiers: none,
        },
        Command::Click {
            at: Location::Point(at),
            button: Button::Left,
            count: 1,
            modifiers: none,
        },
        Command::Scroll {
            at: editor(),
            dx: 0.0,
            dy: -40.0,
            modifiers: none,
        },
    ]);
}

#[test]
fn query_and_control_commands_round_trip() {
    round_trip_commands(vec![
        Command::ListTargets,
        Command::ReadEditorText { start: 3, len: 40 },
        Command::Quit,
        Command::Query {
            what: Query::AppState,
        },
        Command::Query {
            what: Query::FrameTimes,
        },
        Command::Query { what: Query::Theme },
        Command::SetTheme {
            id: "catppuccin-mocha".into(),
        },
        Command::Subscribe {
            topic: Topic::Paint,
        },
        Command::Unsubscribe {
            topic: Topic::Input,
        },
        Command::Await {
            topic: Topic::Frames,
            timeout_ms: 500,
        },
    ]);
}

#[test]
fn every_event_round_trips() {
    let events = vec![
        Event::Frames(FrameSample {
            frame: 7,
            render_us: 100,
            paint_us: 900,
            interval_us: 16_000,
        }),
        Event::Paint {
            frame: 1,
            consistent: false,
            trace: "x".into(),
        },
        Event::Memory(MemoryInfo {
            rss_kib: 5,
            ..Default::default()
        }),
        Event::Input {
            description: "click".into(),
        },
    ];
    for event in events {
        roundtrip(Message::Event {
            event: event.clone(),
        });
        roundtrip(Message::Response {
            req: 8,
            answer: Answer::Event { event },
        });
    }
}

#[test]
fn every_answer_round_trips() {
    let target = Target {
        name: "a".into(),
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
        visible: true,
    };
    let answers = vec![
        Answer::Ok,
        Answer::Error {
            message: "no".into(),
        },
        Answer::Targets {
            targets: vec![target],
        },
        Answer::Memory(MemoryInfo {
            rss_kib: 150_000,
            heap_in_use_mib: Some(90),
            ..Default::default()
        }),
        Answer::FrameTimes {
            frames: vec![
                FrameSample::default(),
                FrameSample {
                    frame: 2,
                    render_us: 1,
                    paint_us: 2,
                    interval_us: 3,
                },
            ],
        },
        Answer::EditorState(EditorState {
            note: Some("A".into()),
            cursor: 3,
            selection: Some((1, 3)),
            len: 9,
            lines: 2,
            scroll_y: -12.5,
            focused: true,
        }),
        Answer::EditorState(EditorState::default()),
        Answer::AppState(AppState {
            note: Some("A".into()),
            tabs: vec!["A".into(), "B".into()],
            active_tab: Some(0),
            words: 4,
            chars: 21,
            links: 3,
            broken_links: 1,
            sidebar_tab: "search".into(),
            search_query: "q".into(),
            search_results: 2,
            sidebar_width: 220.0,
            backlinks_width: 300.5,
            dialog_open: true,
            error: None,
        }),
        Answer::Theme(ThemeState {
            id: "jade-light".into(),
            kind: "light".into(),
            background: "#ffffff".into(),
            text: "#000000".into(),
            accent: "#7287fd".into(),
        }),
        Answer::Text {
            start: 1,
            text: "hé".into(),
            total_len: 9,
        },
        Answer::WindowSize {
            width: 800.0,
            height: 600.0,
        },
    ];
    for (req, answer) in answers.into_iter().enumerate() {
        roundtrip(Message::Response {
            req: req as u64,
            answer,
        });
    }
}

#[test]
fn hand_written_requests_use_defaults() {
    let request: Request =
        decode_line(r#"{"req":9,"cmd":"click","at":{"point":{"x":10,"y":20}}}"#).unwrap();
    assert_eq!(request.req, 9);
    assert_eq!(
        request.command,
        Command::Click {
            at: Location::Point(Point { x: 10.0, y: 20.0 }),
            button: Button::Left,
            count: 1,
            modifiers: Modifiers::default(),
        }
    );
    let key: Request = decode_line(r#"{"req":1,"cmd":"key","key":"a"}"#).unwrap();
    assert_eq!(
        key.command,
        Command::Key {
            key: "a".into(),
            modifiers: Modifiers::default()
        }
    );
}

#[test]
fn event_topic_matches_its_variant() {
    assert_eq!(Event::Memory(MemoryInfo::default()).topic(), Topic::Memory);
    assert_eq!(
        Event::Input {
            description: String::new()
        }
        .topic(),
        Topic::Input
    );
}

#[test]
fn malformed_and_oversized_lines_are_errors() {
    assert!(matches!(
        decode_line::<Request>("{not json"),
        Err(LineError::Json(_))
    ));
    assert!(matches!(
        decode_line::<Request>(r#"{"req":1,"cmd":"bogus"}"#),
        Err(LineError::Json(_))
    ));
    let long = " ".repeat(MAX_LINE_BYTES + 1);
    assert!(matches!(
        decode_line::<Request>(&long),
        Err(LineError::TooLong(_))
    ));
}

#[test]
fn pixel_commands_round_trip() {
    let rect = Rect {
        x: 1.0,
        y: 2.0,
        w: 30.0,
        h: 4.5,
    };
    round_trip_commands(vec![
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
            rect: Some(rect),
            tolerance: 6,
        },
        Command::Expect {
            target: "editor".into(),
            checks: vec![
                Check::InkIn {
                    rect,
                    min_percent: 0.5,
                },
                Check::Empty {
                    rect,
                    max_percent: 0.0,
                },
                Check::Color {
                    x: 1.0,
                    y: 1.0,
                    color: "#1e1e2e".into(),
                    tolerance: 4,
                },
                Check::BboxWithin { rect },
            ],
            tolerance: DEFAULT_TOLERANCE,
        },
        Command::Capture {
            target: "editor".into(),
            rect: None,
            scale: Some(0.5),
        },
    ]);
}

#[test]
fn omitted_pixel_options_take_their_defaults() {
    let facts: Command =
        serde_json::from_str(r#"{"cmd":"facts","target":"t","facts":[{"fact":"bbox"}]}"#).unwrap();
    assert!(matches!(
        facts,
        Command::Facts {
            rect: None,
            tolerance: DEFAULT_TOLERANCE,
            ..
        }
    ));
    let check: Check =
        serde_json::from_str(r#"{"check":"empty","rect":{"x":0,"y":0,"w":1,"h":1}}"#).unwrap();
    assert!(matches!(check, Check::Empty { max_percent, .. } if max_percent == 0.0));
}

#[test]
fn pixel_answers_and_chunks_round_trip() {
    let rect = Rect {
        x: 0.0,
        y: 1.0,
        w: 2.0,
        h: 3.0,
    };
    roundtrip(Message::Response {
        req: 1,
        answer: Answer::Facts(FactsReply {
            width: 10.0,
            height: 20.0,
            scale: 2.0,
            background: "#ffffff".into(),
            facts: vec![
                FactValue::InkGrid {
                    cols: 2,
                    rows: 1,
                    cells: vec![0, 255],
                },
                FactValue::RowProfile { counts: vec![1, 2] },
                FactValue::ColProfile { counts: vec![3] },
                FactValue::Bbox { rect: Some(rect) },
                FactValue::Bbox { rect: None },
                FactValue::Palette {
                    colors: vec![PaletteEntry {
                        color: "#000000".into(),
                        percent: 12.5,
                    }],
                },
                FactValue::Pixel {
                    color: "#102030".into(),
                },
            ],
        }),
    });
    roundtrip(Message::Response {
        req: 2,
        answer: Answer::Expect {
            passed: false,
            results: vec![CheckResult {
                passed: false,
                detail: "ink 0.0%".into(),
            }],
        },
    });
    roundtrip(Message::Response {
        req: 3,
        answer: Answer::Capture(CaptureInfo {
            width: 4,
            height: 5,
            scale: 2.0,
            bounds: rect,
            bytes: 100,
            chunks: 1,
        }),
    });
    roundtrip(Message::Chunk {
        req: 3,
        index: 0,
        total: 1,
        data: "AAAA".into(),
    });
}

#[test]
fn a_full_chunk_fits_a_line_with_room_to_spare() {
    let data = base64::encode(&vec![0xAB; CHUNK_BYTES]);
    let line = encode_line(&Message::Chunk {
        req: u64::MAX,
        index: u32::MAX,
        total: u32::MAX,
        data,
    })
    .unwrap();
    assert!(line.len() < MAX_LINE_BYTES, "{} bytes", line.len());
    assert!(line.len() < MAX_LINE_BYTES - 1024);
}
