//! What the fuzzer does: the actions, and their random but repeatable choice.
//! An action depends only on the random numbers, never on what the app
//! answered, so any part of a sequence can be replayed. Places are therefore
//! given in thousandths of the editor pane, not in pixels.

use std::fmt;

use jade_debug_protocol::Modifiers;

use super::rng::Rng;

/// A place in the editor pane, in thousandths of its width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Spot {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    Type(String),
    Paste(String),
    /// A key with its modifiers, as in `ctrl-shift-left`.
    Key(&'static str),
    Click {
        at: Spot,
        count: u8,
        modifiers: Modifiers,
    },
    Drag {
        from: Spot,
        to: Spot,
    },
    /// Wheel scroll in pixels; negative scrolls down.
    Scroll(i32),
    /// A new window size.
    Resize(u32, u32),
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Type(text) => write!(f, "type {text:?}"),
            Step::Paste(text) => write!(f, "paste {text:?}"),
            Step::Key(chord) => write!(f, "key {chord}"),
            Step::Click {
                at,
                count,
                modifiers,
            } => {
                write!(f, "click {}‰,{}‰ x{count}", at.x, at.y)?;
                if modifiers.ctrl {
                    f.write_str(" ctrl")?;
                }
                if modifiers.shift {
                    f.write_str(" shift")?;
                }
                Ok(())
            }
            Step::Drag { from, to } => {
                write!(f, "drag {}‰,{}‰ to {}‰,{}‰", from.x, from.y, to.x, to.y)
            }
            Step::Scroll(dy) => write!(f, "scroll {dy}"),
            Step::Resize(w, h) => write!(f, "resize {w}x{h}"),
        }
    }
}

/// Pieces of text: words, and Markdown syntax that is often left open or put
/// where it does not belong. No link may point at a file that exists besides
/// the note itself: opening one is not implemented on the headless platform.
const PIECES: &[&str] = &[
    "a",
    "word ",
    "two words ",
    " ",
    " ",
    "\n",
    "\n",
    "\n\n",
    "é",
    "漢字 ",
    "😀",
    "\t",
    "\\",
    "# ",
    "### ",
    "- ",
    "- [ ] ",
    "- [x] ",
    "1. ",
    "  ",
    "> ",
    "> [!note] ",
    "> [!tip]- ",
    "+",
    "-",
    "---\n",
    "===\n",
    "```",
    "```rust\n",
    "~~~",
    "`",
    "$",
    "$$",
    "$$\n",
    "x^2_i ",
    "\\frac{a}{b}",
    "\\newcommand{\\a}{b}",
    "**",
    "*",
    "_",
    "~~",
    "[[",
    "]]",
    "[[Note]]",
    "[[Note#a|b]]",
    "[t](u)",
    "<https://example.invalid>",
    "![[i.png]]",
    "![i](i.png)",
    "|",
    " | ",
    "| a | b |\n| --- | --- |\n",
    "| 1 | 2 |\n",
    "<br>",
    "::: columns\n\n",
    "::: aside left\n\n",
    "\n---\n\n",
    "\n:::\n\n",
    "<jade-columns>\n",
    "</jade-columns>\n",
    "<!-- c -->",
];

/// Whole blocks, for the note a run starts with.
const BLOCKS: &[&str] = &[
    "# A heading",
    "A paragraph with **bold**, *italic*, `code` and a [[Note]] link. It is long enough to \
     wrap onto a second row in a narrow window.",
    "- one\n- two\n    - nested\n- [ ] a task",
    "> a quote\n> on two lines",
    "> [!note]- A folded callout\n> with a body",
    "> [!warning] An open callout\n> body\n> - with a list",
    "```rust\nfn main() {}\n```",
    "$$\n\\sum_{i=0}^{n} x_i\n$$",
    "| Name | Notes |\n| --- | --- |\n| a | $x$ and more words in a cell |\n| b |  |",
    "::: columns\n\nleft side\n\n---\n\nright side\n\n:::",
    "::: aside right\n\n| k | v |\n| - | - |\n| 1 | 2 |\n\n---\n\ntext beside the table\n\n:::",
    "---",
    "![[missing.png]]",
    "Inline $a^2 + b^2$ and a very-long-word-without-any-space-to-break-it-at-all-here.",
];

const MOVES: &[&str] = &[
    "left",
    "right",
    "up",
    "down",
    "home",
    "end",
    "ctrl-left",
    "ctrl-right",
    "shift-left",
    "shift-right",
    "shift-up",
    "shift-down",
    "shift-home",
    "shift-end",
    "ctrl-shift-left",
    "ctrl-shift-right",
];
const EDITS: &[&str] = &[
    "backspace",
    "backspace",
    "backspace",
    "delete",
    "delete",
    "enter",
    "enter",
    "tab",
    "shift-tab",
];
const COMMANDS: &[&str] = &[
    "ctrl-a",
    "ctrl-c",
    "ctrl-x",
    "ctrl-v",
    "ctrl-z",
    "ctrl-z",
    "ctrl-z",
    "ctrl-shift-z",
    "ctrl-/",
];

/// Most pieces one typed or pasted text is made of.
const MAX_TYPED_PIECES: u32 = 4;
const MAX_PASTED_PIECES: u32 = 12;
const MAX_NOTE_BLOCKS: u32 = 14;
const MAX_SCROLL: u32 = 900;
/// The two sidebars take about 460 px and do not give way: in a window much
/// narrower than this the editor has no width left to test.
const MIN_WINDOW_WIDTH: u32 = 800;

fn text(rng: &mut Rng, pieces: u32) -> String {
    let count = rng.between(1, pieces);
    (0..count).map(|_| *rng.pick(PIECES)).collect()
}

fn spot(rng: &mut Rng) -> Spot {
    // Inside the edges, so a place never rounds out of the pane.
    Spot {
        x: rng.between(5, 995),
        y: rng.between(5, 995),
    }
}

/// The note a run starts with.
pub(super) fn note(rng: &mut Rng) -> String {
    let count = rng.between(1, MAX_NOTE_BLOCKS);
    let blocks: Vec<&str> = (0..count).map(|_| *rng.pick(BLOCKS)).collect();
    blocks.join("\n\n")
}

pub(super) fn step(rng: &mut Rng) -> Step {
    match rng.below(100) {
        0..30 => Step::Type(text(rng, MAX_TYPED_PIECES)),
        30..35 => Step::Paste(text(rng, MAX_PASTED_PIECES)),
        35..50 => Step::Key(rng.pick(MOVES)),
        50..62 => Step::Key(rng.pick(EDITS)),
        62..72 => Step::Key(rng.pick(COMMANDS)),
        72..86 => Step::Click {
            at: spot(rng),
            count: *rng.pick(&[1, 1, 1, 1, 2, 3]),
            modifiers: Modifiers {
                ctrl: rng.chance(10),
                shift: rng.chance(15),
                ..Default::default()
            },
        },
        86..90 => Step::Drag {
            from: spot(rng),
            to: spot(rng),
        },
        90..97 => {
            let by = rng.between(1, MAX_SCROLL) as i32;
            Step::Scroll(if rng.chance(60) { -by } else { by })
        }
        _ => Step::Resize(rng.between(MIN_WINDOW_WIDTH, 1400), rng.between(300, 900)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64) -> (String, Vec<Step>) {
        let mut rng = Rng::new(seed);
        let note = note(&mut rng);
        (note, (0..2000).map(|_| step(&mut rng)).collect())
    }

    #[test]
    fn a_seed_gives_the_same_run_and_another_seed_another() {
        assert_eq!(run(7), run(7));
        assert_ne!(run(7).1, run(8).1);
    }

    #[test]
    fn every_kind_of_step_is_chosen_with_sane_values() {
        let (_, steps) = run(1);
        let mut kinds = std::collections::HashSet::new();
        for step in &steps {
            kinds.insert(std::mem::discriminant(step));
            match step {
                Step::Type(text) | Step::Paste(text) => assert!(!text.is_empty()),
                Step::Click { at, .. } => assert!(at.x < 1000 && at.y < 1000),
                Step::Scroll(dy) => assert!(*dy != 0 && dy.unsigned_abs() <= MAX_SCROLL),
                Step::Key(_) | Step::Drag { .. } | Step::Resize(..) => {}
            }
        }
        assert_eq!(kinds.len(), 7);
    }
}
