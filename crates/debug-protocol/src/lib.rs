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
        at: Point,
        #[serde(default)]
        modifiers: Modifiers,
    },
    MouseDown {
        at: Point,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Modifiers,
    },
    MouseUp {
        at: Point,
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
        at: Point,
        dx: f32,
        dy: f32,
        #[serde(default)]
        modifiers: Modifiers,
    },
    ListTargets,
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

/// A named, clickable element and its last-painted bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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
    FrameTimes {
        recent_paint_us: Vec<u32>,
    },
    EditorState {
        note: Option<String>,
        cursor: usize,
        len: usize,
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
    Frames {
        frame: u64,
        paint_us: u32,
    },
    Input {
        description: String,
    },
}

impl Event {
    pub fn topic(&self) -> Topic {
        match self {
            Event::Paint { .. } => Topic::Paint,
            Event::Memory(_) => Topic::Memory,
            Event::Frames { .. } => Topic::Frames,
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

    #[test]
    fn every_command_round_trips() {
        let at = Point { x: 1.5, y: 2.0 };
        let commands = vec![
            Command::Type {
                text: "hi\n".into(),
            },
            Command::Key {
                key: "enter".into(),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            Command::MouseMove {
                at,
                modifiers: Modifiers::default(),
            },
            Command::MouseDown {
                at,
                button: Button::Right,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
            Command::MouseUp {
                at,
                button: Button::Middle,
                modifiers: Modifiers::default(),
            },
            Command::Click {
                at: Location::Target("sidebar.new-note".into()),
                button: Button::Left,
                count: 2,
                modifiers: Modifiers::default(),
            },
            Command::Click {
                at: Location::Point(at),
                button: Button::Left,
                count: 1,
                modifiers: Modifiers::default(),
            },
            Command::Scroll {
                at,
                dx: 0.0,
                dy: -40.0,
                modifiers: Modifiers::default(),
            },
            Command::ListTargets,
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
        ];
        for (req, command) in commands.into_iter().enumerate() {
            roundtrip(Request {
                req: req as u64,
                command,
            });
        }
    }

    #[test]
    fn every_event_round_trips() {
        let events = vec![
            Event::Frames {
                frame: 7,
                paint_us: 900,
            },
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
                recent_paint_us: vec![100, 200],
            },
            Answer::EditorState {
                note: None,
                cursor: 3,
                len: 9,
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
