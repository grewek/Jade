//! The scenario files, `crates/app/tests/acceptance/<area>/<id>.toml`. A
//! scenario is data, not code against an editor API, so the same file runs
//! against the old and the new rendering path (#138).

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Headless window size, in logical pixels, when a scenario names none.
const DEFAULT_SIZE: &str = "1100x700";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scenario {
    pub id: String,
    /// `SPEC.md` rule IDs; empty when no rule covers the behaviour yet.
    pub spec: Vec<String>,
    /// Fix commit hash or issue.
    pub source: String,
    /// The #130 stage that implements the scenario's block.
    pub from_stage: u8,
    #[serde(default = "default_size")]
    pub size: String,
    /// `config.toml` for the run; empty means the defaults.
    #[serde(default)]
    pub config: String,
    /// `false` leaves the opened note without keyboard focus, as before the
    /// first click in it.
    #[serde(default = "yes")]
    pub focus: bool,
    pub note: String,
    #[serde(default, rename = "step")]
    pub steps: Vec<Step>,
    #[serde(default, rename = "expect")]
    pub expects: Vec<Expect>,
    /// The folder the file lies in, filled by the loader.
    #[serde(skip)]
    pub area: String,
}

fn yes() -> bool {
    true
}

fn default_size() -> String {
    DEFAULT_SIZE.into()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    Click(ClickAt),
    /// A key, with modifiers as in `ctrl-down`.
    Key(String),
    Type(String),
    Paste(String),
    /// A dead key or input method: `preview` shows while composing, then
    /// `commit` takes its place.
    Compose {
        preview: String,
        commit: String,
    },
    /// Press at `from`, move by (`dx`, `dy`) with the button held, release.
    Drag {
        from: ClickAt,
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
    },
    Undo {},
    Redo {},
    Scroll {
        dy: f32,
    },
    /// New window size, `WxH`.
    Resize(String),
    /// A view toggle by name (`show-structure`).
    Toggle(String),
}

/// A named element, or the place where the caret would be drawn at a
/// zero-based line and column (in characters) of the source.
/// A virtual empty line (`SPEC.md` §10.1) is addressed, here and in a `caret`
/// expectation, as the source line that typing on it would create. Above the
/// first line that is also line 0, so only typing tells the two apart.
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum ClickAt {
    Target { target: String },
    Text { line: usize, col: usize },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Expect {
    Caret {
        line: usize,
        col: usize,
    },
    /// The whole note.
    Text(String),
    TextContains(String),
    /// The kind of block at a line, as the new path reports it.
    Block {
        line: usize,
        kind: String,
    },
    /// The last step moved neither the scroll position nor the first line on
    /// screen.
    ViewStable(bool),
    /// The last step moved none of these zero-based lines.
    LineStable(Vec<usize>),
    /// The last step moved the content down by this many pixels (negative:
    /// up): every top-level line drawn both before and after it.
    ContentMoved(f32),
    /// The caret is drawn entirely inside the editor's pane.
    CaretVisible(bool),
    NoOverlap(bool),
    /// Slowest frame of the last step.
    MaxFrameMs(f32),
    /// `jade-debug expect` checks (`ink:`, `empty:`, `color:`, `within:`),
    /// separated by spaces.
    Pixels {
        target: String,
        expect: String,
    },
}

pub(super) fn parse_size(text: &str) -> Result<(f32, f32), String> {
    text.split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .filter(|&(w, h): &(f32, f32)| w >= 100.0 && h >= 100.0)
        .ok_or_else(|| format!("{text:?} is not a window size (WxH, at least 100x100)"))
}

pub(super) fn parse(text: &str) -> Result<Scenario, String> {
    let scenario: Scenario = toml::from_str(text).map_err(|err| err.to_string())?;
    parse_size(&scenario.size)?;
    if scenario.expects.is_empty() {
        return Err("a scenario needs at least one [[expect]]".into());
    }
    if !(1..=10).contains(&scenario.from_stage) {
        return Err(format!("from_stage {} is not a stage", scenario.from_stage));
    }
    Ok(scenario)
}

pub(super) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/acceptance")
}

/// Every scenario under [`root`], sorted by area and id. Any file that does
/// not load is an error: a skipped file would look like a passing one.
pub(super) fn load_all() -> Result<Vec<Scenario>, String> {
    let mut scenarios = Vec::new();
    for area in entries(&root())?.into_iter().filter(|p| p.is_dir()) {
        for path in entries(&area)? {
            let fail = |err: String| format!("{}: {err}", path.display());
            let text = std::fs::read_to_string(&path).map_err(|err| fail(err.to_string()))?;
            let mut scenario = parse(&text).map_err(fail)?;
            let stem = path.file_stem().and_then(|s| s.to_str());
            if stem != Some(scenario.id.as_str()) {
                return Err(fail(format!(
                    "the id {:?} is not the file name",
                    scenario.id
                )));
            }
            scenario.area = area
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            scenarios.push(scenario);
        }
    }
    let mut ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(format!("two scenarios are called {:?}", pair[0]));
    }
    Ok(scenarios)
}

fn entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let fail = |err: std::io::Error| format!("{}: {err}", dir.display());
    let mut paths = std::fs::read_dir(dir)
        .map_err(fail)?
        .map(|entry| entry.map(|e| e.path()).map_err(fail))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}
