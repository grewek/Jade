//! Wire protocol of Jade's debug socket: newline-delimited JSON. A client sends
//! [`Request`]s; the app answers each with a [`Message::Response`] carrying the
//! same `req` id and pushes [`Message::Event`]s to subscribed clients. No GPUI
//! dependency, so the app, test harness and CLI clients share these types.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Longest line either side accepts; bounds buffering on malformed input.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

pub type ReqId = u64;

/// Window coordinates in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

/// Where a mouse command lands: raw coordinates, or a named element whose
/// last-painted bounds the app resolves to their center.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Point(Point),
    Target(String),
}

/// A stream of pushed [`Event`]s a client can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// The editor's paint trace (one event per frame that differs).
    Paint,
    /// Periodic RSS samples.
    Memory,
    /// Per-frame paint durations.
    Frames,
    /// Every injected or real input event the window handles.
    Input,
}

/// A one-shot question answered by [`Answer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Query {
    Memory,
    /// Selected note, sidebar tab, search query, open dialog, vault error.
    AppState,
    FrameTimes,
    EditorState,
    WindowSize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Types text, one keystroke per character (`\n` and `\t` are Enter and Tab).
    Type {
        text: String,
    },
    /// Presses a named key (`"enter"`, `"tab"`, `"a"` with modifiers) as a bare
    /// key-down and key-up, like the real platform: no text is produced.
    Key {
        key: String,
        #[serde(default)]
        modifiers: Modifiers,
    },
    MouseMove {
        at: Location,
        #[serde(default)]
        modifiers: Modifiers,
    },
    MouseDown {
        at: Location,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Modifiers,
    },
    MouseUp {
        at: Location,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Modifiers,
    },
    /// Move, down and up in one command.
    Click {
        at: Location,
        #[serde(default)]
        button: Button,
        /// 1 = single, 2 = double click.
        #[serde(default = "one")]
        count: u8,
        #[serde(default)]
        modifiers: Modifiers,
    },
    Scroll {
        at: Location,
        dx: f32,
        dy: f32,
        #[serde(default)]
        modifiers: Modifiers,
    },
    ListTargets,
    /// Reads up to `MAX_TEXT_BYTES` of the open note's text from byte `start`
    /// (rounded down to a character boundary; `len` counts from the rounded
    /// start and is cut at one too), to check what typing produced.
    ReadEditorText {
        start: usize,
        len: usize,
    },
    /// Closes the app cleanly (flushing the open note) after replying.
    Quit,
    Query {
        what: Query,
    },
    Subscribe {
        topic: Topic,
    },
    Unsubscribe {
        topic: Topic,
    },
    /// Replies with the next event on `topic`, or an error after `timeout_ms`.
    Await {
        topic: Topic,
        timeout_ms: u32,
    },
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub req: ReqId,
    #[serde(flatten)]
    pub command: Command,
}

/// A named element and its last-painted bounds (window coordinates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Whether any part lies inside its ancestors' clip. Clicks aim at the
    /// center of the visible part.
    pub visible: bool,
}

/// Longest slice `Command::ReadEditorText` returns.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Frames the app remembers for `Query::FrameTimes`.
pub const FRAME_HISTORY: usize = 240;
/// 60 fps, the per-frame ceiling (`CLAUDE.md` Performance).
pub const FRAME_BUDGET_US: u32 = 16_660;

/// One editor frame: building it (`render_us`, layout and measuring) plus
/// painting it (`paint_us`). `interval_us` is the time since the previous
/// frame started, so a long gap means the window was idle, not slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameSample {
    pub frame: u64,
    pub render_us: u32,
    pub paint_us: u32,
    pub interval_us: u32,
}

impl FrameSample {
    pub fn total_us(&self) -> u32 {
        self.render_us.saturating_add(self.paint_us)
    }
}

/// App-level state that is not in the editor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppState {
    pub note: Option<String>,
    /// `"vault"` or `"search"`.
    pub sidebar_tab: String,
    pub search_query: String,
    pub search_results: usize,
    pub dialog_open: bool,
    /// Why the vault failed to open, if it did.
    pub error: Option<String>,
}

/// What the open note's editor currently holds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EditorState {
    /// Id of the selected note; `None` when no note is open.
    pub note: Option<String>,
    pub cursor: usize,
    /// Selected byte range, if any.
    pub selection: Option<(usize, usize)>,
    pub len: usize,
    pub lines: usize,
    pub scroll_y: f32,
    pub focused: bool,
}

/// Process memory; the heap figures come from glibc and are absent elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub rss_kib: u64,
    #[serde(default)]
    pub heap_in_use_mib: Option<u64>,
    #[serde(default)]
    pub heap_free_retained_mib: Option<u64>,
    #[serde(default)]
    pub mmapped_mib: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Ok,
    Error {
        message: String,
    },
    Targets {
        targets: Vec<Target>,
    },
    Memory(MemoryInfo),
    /// Oldest first; the app keeps the last `FRAME_HISTORY` frames.
    FrameTimes {
        frames: Vec<FrameSample>,
    },
    EditorState(EditorState),
    AppState(AppState),
    /// Reply to `ReadEditorText`: `text` starts at byte `start`; `total_len`
    /// is the whole note, so `start + text.len() < total_len` means more.
    Text {
        start: usize,
        text: String,
        total_len: usize,
    },
    WindowSize {
        width: f32,
        height: f32,
    },
    /// Reply to `Await`: the event that arrived.
    Event {
        event: Event,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "topic", rename_all = "snake_case")]
pub enum Event {
    Paint {
        frame: u64,
        consistent: bool,
        trace: String,
    },
    Memory(MemoryInfo),
    Frames(FrameSample),
    Input {
        description: String,
    },
}

impl Event {
    pub fn topic(&self) -> Topic {
        match self {
            Event::Paint { .. } => Topic::Paint,
            Event::Memory(_) => Topic::Memory,
            Event::Frames(_) => Topic::Frames,
            Event::Input { .. } => Topic::Input,
        }
    }
}

/// Everything the app sends to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    Response { req: ReqId, answer: Answer },
    Event { event: Event },
}

#[derive(Debug)]
pub enum LineError {
    TooLong(usize),
    Json(serde_json::Error),
}

/// Serializes `value` as one line, newline included.
pub fn encode_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(value)?;
    assert!(
        !line.contains('\n'),
        "serde_json emits compact single-line JSON"
    );
    line.push('\n');
    Ok(line)
}

/// Parses one line (with or without its trailing newline).
pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, LineError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(LineError::TooLong(line.len()));
    }
    serde_json::from_str(line.trim_end()).map_err(LineError::Json)
}

#[cfg(test)]
mod tests {
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
                sidebar_tab: "search".into(),
                search_query: "q".into(),
                search_results: 2,
                dialog_open: true,
                error: None,
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
}
