//! Wire protocol of Jade's debug socket: newline-delimited JSON. A client sends
//! [`Request`]s; the app answers each with a [`Message::Response`] carrying the
//! same `req` id and pushes [`Message::Event`]s to subscribed clients. No GPUI
//! dependency, so the app, test harness and CLI clients share these types.

pub mod base64;

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
    /// The active theme: id, kind and a few resolved colors.
    Theme,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Switches the running app to theme `id` without touching `config.toml`.
    SetTheme {
        id: String,
    },
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
        /// The button held during the move; set it to feed a drag.
        #[serde(default)]
        pressed: Option<Button>,
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
    /// Press at `from`, move by (`dx`, `dy`) in `steps` equal held-button moves,
    /// release there.
    Drag {
        from: Location,
        dx: f32,
        dy: f32,
        #[serde(default = "default_drag_steps")]
        steps: u8,
        #[serde(default)]
        button: Button,
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
    /// Measures `facts` on the pixels of the element `target` (or `rect` of it)
    /// as last painted. Needs the headless mode (see `jade-debug launch`).
    Facts {
        target: String,
        facts: Vec<Fact>,
        #[serde(default)]
        rect: Option<Rect>,
        #[serde(default = "default_tolerance")]
        tolerance: u8,
    },
    /// Checks the `checks` against the pixels of the element `target`.
    Expect {
        target: String,
        checks: Vec<Check>,
        #[serde(default = "default_tolerance")]
        tolerance: u8,
    },
    /// A PNG of the element `target` (or `rect` of it), optionally scaled by
    /// `scale` (0 < scale <= 1), sent as chunks after the reply.
    Capture {
        target: String,
        #[serde(default)]
        rect: Option<Rect>,
        #[serde(default)]
        scale: Option<f32>,
    },
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

/// Held-button moves a `Drag` makes when `steps` is omitted.
pub const DEFAULT_DRAG_STEPS: u8 = 8;

fn default_drag_steps() -> u8 {
    DEFAULT_DRAG_STEPS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub req: ReqId,
    #[serde(flatten)]
    pub command: Command,
}

/// A rectangle in logical pixels. Relative to an element's top-left in
/// `Facts` and `Expect`, in window coordinates in `CaptureInfo::bounds`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Default per-channel difference above which a pixel counts as ink.
pub const DEFAULT_TOLERANCE: u8 = 8;

fn default_tolerance() -> u8 {
    DEFAULT_TOLERANCE
}

/// Most cells an `InkGrid` may ask for; bounds the reply.
pub const MAX_GRID_CELLS: u32 = 4096;
/// Longest row or column profile a reply carries (logical pixels).
pub const MAX_PROFILE_LEN: u32 = 4096;
/// Palette entries a reply carries at most.
pub const MAX_PALETTE: u8 = 32;
/// Raw bytes in one capture chunk; base64 makes the line about a third longer,
/// still far below `MAX_LINE_BYTES`.
pub const CHUNK_BYTES: usize = 32 * 1024;
/// Largest PNG a capture sends; a bigger one is an error that says to crop or
/// scale.
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// One fact the app computes from an element's rendered pixels. "Ink" is a
/// pixel that differs from the region's background (its most common colour) by
/// more than the request's tolerance in any channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum Fact {
    /// Ink coverage (0..=255) of each cell of a `cols` x `rows` grid laid over
    /// the region, row by row: a thumbnail of the layout.
    InkGrid { cols: u16, rows: u16 },
    /// Ink pixels in each logical-pixel row: text lines, gaps, overlap.
    RowProfile,
    /// Ink pixels in each logical-pixel column.
    ColProfile,
    /// Bounds of all the ink, relative to the region.
    Bbox,
    /// The most common colours (each channel quantized to 4 bits).
    Palette { max: u8 },
    /// The colour at one point of the region.
    Pixel { x: f32, y: f32 },
}

/// The measured value of one [`Fact`], in the same order as asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum FactValue {
    InkGrid {
        cols: u16,
        rows: u16,
        cells: Vec<u8>,
    },
    RowProfile {
        counts: Vec<u32>,
    },
    ColProfile {
        counts: Vec<u32>,
    },
    /// `None` when the region has no ink.
    Bbox {
        rect: Option<Rect>,
    },
    Palette {
        colors: Vec<PaletteEntry>,
    },
    Pixel {
        color: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteEntry {
    /// `#rrggbb`.
    pub color: String,
    /// Share of the region's pixels, 0..=100.
    pub percent: f32,
}

/// Something the client expects of an element's pixels, written as geometry
/// so no renderer is needed to state it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum Check {
    /// At least `min_percent` of `rect` is ink: something is drawn there.
    InkIn {
        rect: Rect,
        #[serde(default = "default_min_percent")]
        min_percent: f32,
    },
    /// At most `max_percent` of `rect` is ink: it is empty.
    Empty {
        rect: Rect,
        #[serde(default)]
        max_percent: f32,
    },
    /// The pixel at (`x`, `y`) is `color` (`#rrggbb`) within `tolerance`.
    Color {
        x: f32,
        y: f32,
        color: String,
        #[serde(default = "default_tolerance")]
        tolerance: u8,
    },
    /// All the ink lies inside `rect`.
    BboxWithin { rect: Rect },
}

fn default_min_percent() -> f32 {
    0.1
}

/// The verdict on one [`Check`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub passed: bool,
    /// What was measured, so a failure says why.
    pub detail: String,
}

/// What a `Facts` request measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactsReply {
    /// The element's size in logical pixels.
    pub width: f32,
    pub height: f32,
    /// Device pixels per logical pixel of the image the facts come from.
    pub scale: f32,
    /// The region's background colour, `#rrggbb`.
    pub background: String,
    pub facts: Vec<FactValue>,
}

/// What a `Capture` request produced; the PNG follows as `chunks`
/// [`Message::Chunk`]s, base64, to be joined in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureInfo {
    /// Size of the PNG in pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels per logical pixel of the app's frame.
    pub scale: f32,
    /// The captured region in window coordinates (logical pixels).
    pub bounds: Rect,
    /// PNG size in bytes (before base64).
    pub bytes: usize,
    pub chunks: u32,
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
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppState {
    /// The active tab's note.
    pub note: Option<String>,
    /// Note ids of the open tabs, left to right.
    pub tabs: Vec<String>,
    pub active_tab: Option<usize>,
    /// The status bar's counts for the active note (0 with no note).
    pub words: usize,
    pub chars: usize,
    /// Links in the active note (0 with no note).
    pub links: usize,
    /// Of those, how many point at nothing.
    pub broken_links: usize,
    /// `"vault"` or `"search"`.
    pub sidebar_tab: String,
    pub search_query: String,
    pub search_results: usize,
    /// Widths in px of the left sidebar and the backlinks sidebar.
    pub sidebar_width: f32,
    pub backlinks_width: f32,
    pub dialog_open: bool,
    /// Why the vault failed to open, if it did.
    pub error: Option<String>,
}

/// The active theme; colors are `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThemeState {
    pub id: String,
    /// `"light"` or `"dark"`.
    pub kind: String,
    pub background: String,
    pub text: String,
    pub accent: String,
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
    Theme(ThemeState),
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
    Facts(FactsReply),
    /// Reply to `Expect`: `passed` is true when every check did.
    Expect {
        passed: bool,
        results: Vec<CheckResult>,
    },
    Capture(CaptureInfo),
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
    Response {
        req: ReqId,
        answer: Answer,
    },
    Event {
        event: Event,
    },
    /// Part `index` of `total` of a capture's PNG (base64), sent right after
    /// the `Capture` reply to the same `req`.
    Chunk {
        req: ReqId,
        index: u32,
        total: u32,
        data: String,
    },
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
            serde_json::from_str(r#"{"cmd":"facts","target":"t","facts":[{"fact":"bbox"}]}"#)
                .unwrap();
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
}
