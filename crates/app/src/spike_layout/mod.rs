//! SPIKE #134 (throwaway, never merged): a plain-paragraph editor element that
//! takes its width in the layout step of the same frame, changes layout state
//! only in prepaint and keeps paint read-only. Every source line is one
//! paragraph. Enabled with `JADE_SPIKE_LAYOUT=<note path>`.

mod element;
mod input;
mod trace;

#[cfg(test)]
mod tests;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::*;

pub(crate) use element::SpikeElement;

actions!(spike, [SpikeTab, SpikeNarrower, SpikeWider]);

pub(crate) const KEY_CONTEXT: &str = "SpikeEditor";
pub(crate) const FONT_SIZE: Pixels = px(15.);
pub(crate) const LINE_HEIGHT: Pixels = px(22.);
/// Average glyph advance as a share of the font size, for height estimates.
const ADVANCE_RATIO: f32 = 0.48;

pub(crate) fn key_bindings() -> Vec<KeyBinding> {
    vec![
        // Must out-rank the `Root` component's own "tab" binding.
        KeyBinding::new("tab", SpikeTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-alt-left", SpikeNarrower, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-alt-right", SpikeWider, Some(KEY_CONTEXT)),
    ]
}

/// How the element is hosted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    /// In a GPUI scroll `div`: the height depends on the width and comes from
    /// the measure callback.
    Measured,
    /// Fills the pane and scrolls itself (K21): taffy never needs its height.
    Fill,
}

/// What prepaint built; paint, input and the IME only read it.
pub(crate) struct Snapshot {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) lines: Vec<ShapedLine_>,
    pub(crate) caret: Option<Bounds<Pixels>>,
    pub(crate) hitbox: Hitbox,
}

pub(crate) struct ShapedLine_ {
    pub(crate) index: usize,
    /// Window coordinates.
    pub(crate) origin: Point<Pixels>,
    pub(crate) wrapped: WrappedLine,
}

pub(crate) struct SpikeView {
    pub(crate) lines: Vec<String>,
    /// `(line, byte in line)`.
    pub(crate) cursor: (usize, usize),
    /// IME composition, in bytes of the cursor line.
    pub(crate) marked: Option<Range<usize>>,
    pub(crate) mode: Mode,
    pub(crate) focus: FocusHandle,
    /// The width the exact heights belong to.
    pub(crate) width: Pixels,
    /// Exact paragraph heights at `width`; 0 where only an estimate exists.
    pub(crate) exact: Vec<f32>,
    /// `Fill` mode only.
    pub(crate) scroll_y: Pixels,
    pub(crate) snapshot: Option<Rc<Snapshot>>,
    pub(crate) frame: u64,
    pub(crate) tabs_handled: u32,
    /// Tests only: makes paint break its rule, to show the check fires.
    pub(crate) paint_mutates: bool,
}

impl SpikeView {
    pub(crate) fn new(text: &str, mode: Mode, cx: &mut Context<Self>) -> Self {
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        assert!(!lines.is_empty(), "split always yields one line");
        let exact = vec![0.; lines.len()];
        Self {
            lines,
            cursor: (0, 0),
            marked: None,
            mode,
            focus: cx.focus_handle(),
            width: px(0.),
            exact,
            scroll_y: px(0.),
            snapshot: None,
            frame: 0,
            tabs_handled: 0,
            paint_mutates: false,
        }
    }

    /// `JADE_SPIKE_LAYOUT=<note>`, `JADE_SPIKE_MODE=measured|fill`.
    pub(crate) fn from_env(cx: &mut App) -> Option<Entity<Self>> {
        let path = std::env::var_os("JADE_SPIKE_LAYOUT")?;
        let text = match std::fs::read_to_string(Path::new(&path)) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("[spike] cannot read {path:?}: {err}");
                return None;
            }
        };
        let mode = match std::env::var("JADE_SPIKE_MODE").as_deref() {
            Ok("fill") => Mode::Fill,
            _ => Mode::Measured,
        };
        eprintln!("[spike] {} bytes, mode {mode:?}", text.len());
        Some(cx.new(|cx| Self::new(&text, mode, cx)))
    }

    /// One paragraph's height: exact if it was shaped at this width.
    pub(crate) fn line_height_at(&self, index: usize, width: Pixels) -> f32 {
        if width == self.width && self.exact[index] > 0. {
            return self.exact[index];
        }
        let per_row = (f32::from(width) / (f32::from(FONT_SIZE) * ADVANCE_RATIO)).max(1.);
        let rows = (self.lines[index].len() as f32 / per_row).ceil().max(1.);
        rows * f32::from(LINE_HEIGHT)
    }

    /// Linear on purpose: the spike measures what even the naive sum costs.
    pub(crate) fn total_height(&self, width: Pixels) -> Pixels {
        px((0..self.lines.len())
            .map(|i| self.line_height_at(i, width))
            .sum())
    }

    /// The first line reaching below `top` (content coordinates) and its y.
    pub(crate) fn line_at(&self, top: f32, width: Pixels) -> (usize, f32) {
        let mut y = 0.;
        for index in 0..self.lines.len() {
            let height = self.line_height_at(index, width);
            if y + height > top {
                return (index, y);
            }
            y += height;
        }
        let last = self.lines.len() - 1;
        (last, y - self.line_height_at(last, width))
    }

    /// Everything paint must leave alone.
    pub(crate) fn state_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.lines.hash(&mut hasher);
        self.cursor.hash(&mut hasher);
        self.marked.hash(&mut hasher);
        f32::from(self.width).to_bits().hash(&mut hasher);
        f32::from(self.scroll_y).to_bits().hash(&mut hasher);
        for height in &self.exact {
            height.to_bits().hash(&mut hasher);
        }
        self.frame.hash(&mut hasher);
        self.tabs_handled.hash(&mut hasher);
        self.snapshot
            .as_ref()
            .map(|s| Rc::as_ptr(s) as usize)
            .hash(&mut hasher);
        hasher.finish()
    }

    /// Replaces `range` (bytes of the cursor line); newlines split the line.
    pub(crate) fn replace(&mut self, range: Range<usize>, text: &str) {
        let (line, _) = self.cursor;
        let current = &mut self.lines[line];
        assert!(range.start <= range.end && range.end <= current.len());
        let tail = current.split_off(range.end);
        current.truncate(range.start);
        let mut parts = text.split('\n');
        current.push_str(parts.next().unwrap_or(""));
        self.exact[line] = 0.;
        let mut last = line;
        for part in parts {
            last += 1;
            self.lines.insert(last, part.to_string());
            self.exact.insert(last, 0.);
        }
        self.cursor = (last, self.lines[last].len());
        self.lines[last].push_str(&tail);
    }

    pub(crate) fn backspace(&mut self) {
        let (line, byte) = self.cursor;
        if let Some(ch) = self.lines[line][..byte].chars().next_back() {
            self.replace(byte - ch.len_utf8()..byte, "");
        } else if line > 0 {
            let tail = self.lines.remove(line);
            self.exact.remove(line);
            self.cursor = (line - 1, self.lines[line - 1].len());
            self.lines[line - 1].push_str(&tail);
            self.exact[line - 1] = 0.;
        }
    }

    pub(crate) fn step(&mut self, forward: bool) {
        let (line, byte) = self.cursor;
        let text = &self.lines[line];
        let next = if forward {
            text[byte..].chars().next().map(|c| byte + c.len_utf8())
        } else {
            text[..byte]
                .chars()
                .next_back()
                .map(|c| byte - c.len_utf8())
        };
        if let Some(next) = next {
            self.cursor = (line, next);
        }
    }
}

impl Render for SpikeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SpikeElement::new(cx.entity())
    }
}
