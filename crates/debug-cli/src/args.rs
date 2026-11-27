//! Command-line grammar, shared by one-shot invocations and `script` lines.

use std::path::PathBuf;

use jade_debug_protocol::{
    Button, Command, DEFAULT_DRAG_STEPS, DEFAULT_TOLERANCE, Fact, Location, Modifiers, Point,
    Query, Rect, Topic, numbers,
};

pub(crate) const DEFAULT_SOCKET: &str = "/tmp/jade-debug.sock";

pub(crate) const HELP: &str = "\
jade-debug [--socket PATH] <command> [args]    (socket: $JADE_DEBUG_SOCKET or /tmp/jade-debug.sock)

observe
  targets                       named elements, bounds, visibility
  state                         editor: note, cursor, selection, size, scroll, focus
  app                           selected note, sidebar tab, search, dialog open
  text [START] [LEN]            slice of the open note (default 0, 2000)
  frames [--raw]                recent editor frame times vs the 16.66ms budget
  theme [ID]                    active theme, or switch to ID for this run (not saved)
  invariants                    check the caret, the patched state and the frames since the
                                last check (fails and lists what is wrong)
  memory | size                 process memory | window size
  subscribe|unsubscribe TOPIC   topics: paint memory frames input (script only)
  await TOPIC [TIMEOUT_MS]      the next event on TOPIC
  watch TOPIC [SECONDS]         stream events (default 5s)

look (needs `launch --headless`; TARGET is a target name, coordinates are logical px
relative to its top-left, \"ink\" is anything unlike the region's background)
  facts TARGET FACT... [--rect X,Y,W,H] [--tol N]
                                FACT: grid:CxR (ink coverage thumbnail) rows cols
                                (ink per pixel row/column) bbox (bounds of the ink)
                                palette[:N] pixel:X,Y
  expect TARGET CHECK... [--tol N]
                                CHECK: ink:X,Y,W,H[,MIN%] empty:X,Y,W,H[,MAX%]
                                color:X,Y,#rrggbb[,TOL] within:X,Y,W,H
                                prints PASS/FAIL with what was measured; fails if any does
  capture TARGET [--rect X,Y,W,H] [--scale S] [--out FILE.png]
                                a PNG of the element (S in 0..1 shrinks it)

act (WHERE is a target name like note-Big, or x,y)
  click WHERE [--right|--middle] [--double|--triple] [--shift --ctrl --alt --meta]
  move WHERE [--held] | down WHERE | up WHERE   (--held: left button down during the move)
  drag WHERE DX [DY] [STEPS]    press at WHERE, move by DX,DY px in STEPS held moves (default 8), release
  scroll WHERE DY [DX]          negative DY scrolls down
  type TEXT...                  \\n and \\t are Enter and Tab
  key CHORD                     enter, escape, backspace, ctrl+a, shift+left, ...
  quit                          close the app (flushes the open note)

session
  launch VAULT [--bin PATH] [--in-place] [--headless] [--size WxH]
                                start the app on a COPY of VAULT (autosave would
                                otherwise edit your notes) and wait until ready;
                                --headless renders without a window (needed for
                                facts, expect and capture; the app must be built
                                with --features debug-socket)
  script                        read commands from stdin, one per line
                                (# comments, \"quoted words\", sleep MS)

fuzz (starts its own headless apps on throwaway vaults)
  fuzz [--seed N] [--runs N] [--steps N] [--bin PATH] [--trace]
                                random input until the app crashes or hangs; the
                                same seed repeats a run
";

pub(crate) struct Cli {
    pub socket: PathBuf,
    pub action: Action,
}

pub(crate) struct LaunchArgs {
    pub vault: PathBuf,
    pub bin: Option<PathBuf>,
    pub in_place: bool,
    pub headless: bool,
    /// `WxH` in logical pixels, for the headless window.
    pub size: Option<String>,
}

pub(crate) enum Action {
    Send(Command),
    /// A capture: the command, and where the PNG goes.
    Capture {
        command: Command,
        out: PathBuf,
    },
    Frames {
        raw: bool,
    },
    Sleep(u64),
    Watch {
        topic: Topic,
        seconds: u64,
    },
    Script,
    Launch(LaunchArgs),
    Fuzz(crate::fuzz::FuzzArgs),
    Help,
}

pub(crate) fn parse_cli(args: &[String], env_socket: Option<String>) -> Result<Cli, String> {
    let mut socket = env_socket.unwrap_or_else(|| DEFAULT_SOCKET.into());
    let mut rest = args;
    while let [flag, tail @ ..] = rest {
        if flag != "--socket" {
            break;
        }
        let [path, tail @ ..] = tail else {
            return Err("--socket needs a path".into());
        };
        socket = path.clone();
        rest = tail;
    }
    Ok(Cli {
        socket: socket.into(),
        action: parse_action(rest)?,
    })
}

/// Splits flags (`--x`) from positional words.
fn split_flags(words: &[String]) -> (Vec<&str>, Vec<&str>) {
    let (flags, positional): (Vec<_>, Vec<_>) = words
        .iter()
        .map(String::as_str)
        .partition(|w| w.starts_with("--"));
    (flags, positional)
}

fn reject_unknown(flags: &[&str], known: &[&str]) -> Result<(), String> {
    match flags.iter().find(|f| !known.contains(f)) {
        Some(flag) => Err(format!("unknown flag {flag}")),
        None => Ok(()),
    }
}

pub(crate) fn parse_action(words: &[String]) -> Result<Action, String> {
    let Some((name, rest)) = words.split_first() else {
        return Ok(Action::Help);
    };
    let (flags, pos) = split_flags(rest);
    let send = |command| Ok(Action::Send(command));
    match name.as_str() {
        "help" | "--help" | "-h" => Ok(Action::Help),
        "targets" => send(Command::ListTargets),
        "state" => send(Command::Query {
            what: Query::EditorState,
        }),
        "app" => send(Command::Query {
            what: Query::AppState,
        }),
        "theme" => match pos.as_slice() {
            [] => send(Command::Query { what: Query::Theme }),
            [id] => send(Command::SetTheme { id: (*id).into() }),
            _ => Err("theme takes at most one ID".into()),
        },
        "invariants" => send(Command::Query {
            what: Query::Invariants,
        }),
        "memory" => send(Command::Query {
            what: Query::Memory,
        }),
        "size" => send(Command::Query {
            what: Query::WindowSize,
        }),
        "quit" => send(Command::Quit),
        "script" => Ok(Action::Script),
        "fuzz" => crate::fuzz::parse_args(rest).map(Action::Fuzz),
        "frames" => {
            reject_unknown(&flags, &["--raw"])?;
            Ok(Action::Frames {
                raw: flags.contains(&"--raw"),
            })
        }
        "text" => send(Command::ReadEditorText {
            start: number(pos.first(), 0, "START")?,
            len: number(pos.get(1), 2000, "LEN")?,
        }),
        "sleep" => Ok(Action::Sleep(number(pos.first(), 0, "MS")?)),
        "subscribe" => send(Command::Subscribe {
            topic: topic(pos.first())?,
        }),
        "unsubscribe" => send(Command::Unsubscribe {
            topic: topic(pos.first())?,
        }),
        "await" => send(Command::Await {
            topic: topic(pos.first())?,
            timeout_ms: number(pos.get(1), 5000, "TIMEOUT_MS")?,
        }),
        "watch" => Ok(Action::Watch {
            topic: topic(pos.first())?,
            seconds: number(pos.get(1), 5, "SECONDS")?,
        }),
        "facts" => facts_command(&flags_with_values(rest)?),
        "expect" => expect_command(&flags_with_values(rest)?),
        "capture" => capture_command(&flags_with_values(rest)?),
        "type" => type_command(rest),
        "key" => key_command(&pos),
        "launch" => launch_command(rest),
        "click" | "move" | "down" | "up" | "scroll" | "drag" => mouse_command(name, &flags, &pos),
        other => Err(format!("unknown command {other:?} (try `jade-debug help`)")),
    }
}

fn number<T: std::str::FromStr>(word: Option<&&str>, default: T, what: &str) -> Result<T, String> {
    match word {
        None => Ok(default),
        Some(w) => w
            .parse()
            .map_err(|_| format!("{what} must be a number, got {w:?}")),
    }
}

fn topic(word: Option<&&str>) -> Result<Topic, String> {
    match word.copied() {
        Some("paint") => Ok(Topic::Paint),
        Some("memory") => Ok(Topic::Memory),
        Some("frames") => Ok(Topic::Frames),
        Some("input") => Ok(Topic::Input),
        other => Err(format!(
            "topic must be paint|memory|frames|input, got {other:?}"
        )),
    }
}

fn type_command(words: &[String]) -> Result<Action, String> {
    if words.is_empty() {
        return Err("type needs some text".into());
    }
    Ok(Action::Send(Command::Type {
        text: unescape(&words.join(" ")),
    }))
}

pub(crate) fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn key_command(pos: &[&str]) -> Result<Action, String> {
    let [chord] = pos else {
        return Err("key needs exactly one chord, e.g. enter or ctrl+a".into());
    };
    let (key, modifiers) = parse_chord(chord)?;
    Ok(Action::Send(Command::Key { key, modifiers }))
}

pub(crate) fn parse_chord(chord: &str) -> Result<(String, Modifiers), String> {
    let (mods, key) = match chord.strip_suffix('+') {
        Some(head) => (head.strip_suffix('+').unwrap_or(head), "+"),
        None => chord.rsplit_once('+').unwrap_or(("", chord)),
    };
    if key.is_empty() {
        return Err(format!("empty key in chord {chord:?}"));
    }
    let mut modifiers = Modifiers::default();
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "meta" | "super" | "cmd" => modifiers.meta = true,
            other => return Err(format!("unknown modifier {other:?} in {chord:?}")),
        }
    }
    Ok((key.to_ascii_lowercase(), modifiers))
}

fn parse_where(word: &str) -> Location {
    if let Some((x, y)) = word.split_once(',')
        && let (Ok(x), Ok(y)) = (x.parse(), y.parse())
    {
        return Location::Point(Point { x, y });
    }
    Location::Target(word.to_owned())
}

const MOUSE_FLAGS: &[&str] = &[
    "--right", "--middle", "--double", "--triple", "--shift", "--ctrl", "--alt", "--meta", "--held",
];

// One flat mapping from the mouse subcommand names to their actions.
#[allow(clippy::too_many_lines)]
fn mouse_command(name: &str, flags: &[&str], pos: &[&str]) -> Result<Action, String> {
    reject_unknown(flags, MOUSE_FLAGS)?;
    let has = |f| flags.contains(&f);
    let at = parse_where(pos.first().ok_or("a target name or x,y is needed")?);
    let button = match (has("--right"), has("--middle")) {
        (true, _) => Button::Right,
        (_, true) => Button::Middle,
        _ => Button::Left,
    };
    let modifiers = Modifiers {
        ctrl: has("--ctrl"),
        alt: has("--alt"),
        shift: has("--shift"),
        meta: has("--meta"),
    };
    let command = match name {
        "click" => {
            let count = if has("--triple") {
                3
            } else if has("--double") {
                2
            } else {
                1
            };
            Command::Click {
                at,
                button,
                count,
                modifiers,
            }
        }
        "move" => Command::MouseMove {
            at,
            pressed: has("--held").then_some(Button::Left),
            modifiers,
        },
        "drag" => Command::Drag {
            from: at,
            dx: number(pos.get(1), f32::NAN, "DX")?,
            dy: number(pos.get(2), 0.0, "DY")?,
            steps: number(pos.get(3), DEFAULT_DRAG_STEPS, "STEPS")?,
            button,
            modifiers,
        },
        "down" => Command::MouseDown {
            at,
            button,
            modifiers,
        },
        "up" => Command::MouseUp {
            at,
            button,
            modifiers,
        },
        _ => Command::Scroll {
            at,
            dy: number(pos.get(1), f32::NAN, "DY")?,
            dx: number(pos.get(2), 0.0, "DX")?,
            modifiers,
        },
    };
    if let Command::Scroll { dy, .. } = &command
        && dy.is_nan()
    {
        return Err("scroll needs DY".into());
    }
    if let Command::Drag { dx, .. } = &command
        && dx.is_nan()
    {
        return Err("drag needs DX".into());
    }
    Ok(Action::Send(command))
}

fn launch_command(words: &[String]) -> Result<Action, String> {
    let mut vault = None;
    let mut bin = None;
    let mut in_place = false;
    let mut headless = false;
    let mut size = None;
    let mut iter = words.iter();
    while let Some(word) = iter.next() {
        match word.as_str() {
            "--in-place" => in_place = true,
            "--headless" => headless = true,
            "--size" => size = Some(iter.next().ok_or("--size needs WxH")?.clone()),
            "--bin" => bin = Some(PathBuf::from(iter.next().ok_or("--bin needs a path")?)),
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            path if vault.is_none() => vault = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra:?}")),
        }
    }
    let vault = vault.ok_or("launch needs a vault directory")?;
    Ok(Action::Launch(LaunchArgs {
        vault,
        bin,
        in_place,
        headless,
        size,
    }))
}

/// Words of a pixel command: the positional ones, and the `--flag VALUE` pairs.
struct Words<'a> {
    pos: Vec<&'a str>,
    flags: Vec<(&'a str, &'a str)>,
}

impl<'a> Words<'a> {
    fn flag(&self, name: &str) -> Option<&'a str> {
        self.flags.iter().find(|(f, _)| *f == name).map(|(_, v)| *v)
    }

    fn reject_unknown(&self, known: &[&str]) -> Result<(), String> {
        match self.flags.iter().find(|(f, _)| !known.contains(f)) {
            Some((flag, _)) => Err(format!("unknown flag {flag}")),
            None => Ok(()),
        }
    }
}

fn flags_with_values(words: &[String]) -> Result<Words<'_>, String> {
    let mut parsed = Words {
        pos: Vec::new(),
        flags: Vec::new(),
    };
    let mut iter = words.iter().map(String::as_str);
    while let Some(word) = iter.next() {
        if word.starts_with("--") {
            let value = iter.next().ok_or_else(|| format!("{word} needs a value"))?;
            parsed.flags.push((word, value));
        } else {
            parsed.pos.push(word);
        }
    }
    Ok(parsed)
}

/// Comma-separated numbers, exactly `count` of them.
fn rect(text: &str) -> Result<Rect, String> {
    text.parse()
}

fn tolerance(words: &Words) -> Result<u8, String> {
    words.flag("--tol").map_or(Ok(DEFAULT_TOLERANCE), |t| {
        t.parse()
            .map_err(|_| format!("--tol must be 0 to 255, got {t:?}"))
    })
}

fn parse_fact(word: &str) -> Result<Fact, String> {
    let (name, arg) = word
        .split_once(':')
        .map_or((word, None), |(n, a)| (n, Some(a)));
    match (name, arg) {
        ("grid", Some(dims)) => {
            let bad = || format!("grid needs CxR, like grid:32x8, got {dims:?}");
            let (c, r) = dims.split_once('x').ok_or_else(bad)?;
            Ok(Fact::InkGrid {
                cols: c.parse().map_err(|_| bad())?,
                rows: r.parse().map_err(|_| bad())?,
            })
        }
        ("rows", None) => Ok(Fact::RowProfile),
        ("cols", None) => Ok(Fact::ColProfile),
        ("bbox", None) => Ok(Fact::Bbox),
        ("palette", arg) => Ok(Fact::Palette {
            max: arg.map_or(Ok(8), |a| a.parse().map_err(|_| "palette:N needs a number"))?,
        }),
        ("pixel", Some(xy)) => {
            let v = numbers(xy, 2, "pixel:X,Y")?;
            Ok(Fact::Pixel { x: v[0], y: v[1] })
        }
        _ => Err(format!(
            "unknown fact {word:?}: use grid:CxR rows cols bbox palette[:N] pixel:X,Y"
        )),
    }
}

fn facts_command(words: &Words) -> Result<Action, String> {
    words.reject_unknown(&["--rect", "--tol"])?;
    let Some((target, facts)) = words.pos.split_first() else {
        return Err("facts needs a target and at least one fact".into());
    };
    if facts.is_empty() {
        return Err(
            "facts needs at least one fact: grid:CxR rows cols bbox palette pixel:X,Y".into(),
        );
    }
    Ok(Action::Send(Command::Facts {
        target: (*target).into(),
        facts: facts
            .iter()
            .map(|f| parse_fact(f))
            .collect::<Result<_, _>>()?,
        rect: words.flag("--rect").map(rect).transpose()?,
        tolerance: tolerance(words)?,
    }))
}

fn expect_command(words: &Words) -> Result<Action, String> {
    words.reject_unknown(&["--tol"])?;
    let Some((target, checks)) = words.pos.split_first() else {
        return Err("expect needs a target and at least one check".into());
    };
    if checks.is_empty() {
        return Err("expect needs at least one check".into());
    }
    Ok(Action::Send(Command::Expect {
        target: (*target).into(),
        checks: checks.iter().map(|c| c.parse()).collect::<Result<_, _>>()?,
        tolerance: tolerance(words)?,
    }))
}

fn capture_command(words: &Words) -> Result<Action, String> {
    words.reject_unknown(&["--rect", "--scale", "--out"])?;
    let [target] = words.pos.as_slice() else {
        return Err("capture needs exactly one target".into());
    };
    let scale = words
        .flag("--scale")
        .map(|s| {
            s.parse::<f32>()
                .map_err(|_| format!("--scale must be a number, got {s:?}"))
        })
        .transpose()?;
    let out = words.flag("--out").map_or_else(
        || PathBuf::from(format!("/tmp/jade-capture-{target}.png")),
        PathBuf::from,
    );
    Ok(Action::Capture {
        command: Command::Capture {
            target: (*target).into(),
            rect: words.flag("--rect").map(rect).transpose()?,
            scale,
        },
        out,
    })
}

/// Splits a script line into words; double or single quotes group words.
pub(crate) fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unterminated quote".into());
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

#[cfg(test)]
mod tests;
