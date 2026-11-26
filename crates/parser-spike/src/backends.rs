//! The candidates, behind one interface.

use crate::libs::{Comrak, Pulldown, TreeSitter};
use crate::own::Own;
use crate::tree::{BlockParser, Ctx, SegmentParser, Unit, Windowed};

/// The own parser behind the interface the libraries get: what the window
/// strategy costs by itself.
#[derive(Default)]
pub struct OwnSegment(Own);

impl SegmentParser for OwnSegment {
    fn parse_segment(&mut self, segment: &str, base: usize, ctx: Ctx, out: &mut Vec<Unit>) {
        self.0.parse_from(segment, 0, ctx, &mut |mut unit| {
            unit.start += base as u32;
            out.push(unit);
            std::ops::ControlFlow::Continue(())
        });
    }
}

pub type Candidate = (&'static str, fn() -> Box<dyn BlockParser + Send>);

pub const CANDIDATES: &[Candidate] = &[
    ("own", || Box::new(Own)),
    ("own-windowed", || {
        Box::new(Windowed::new(OwnSegment::default()))
    }),
    ("pulldown", || Box::new(Windowed::new(Pulldown))),
    ("comrak", || Box::new(Windowed::new(Comrak::new()))),
    ("tree-sitter", || Box::new(Windowed::new(TreeSitter::new()))),
];

/// The libraries on a whole string with no pre-pass: what they handle alone.
pub fn raw() -> Vec<(&'static str, Box<dyn SegmentParser>)> {
    vec![
        ("pulldown raw", Box::new(Pulldown)),
        ("comrak raw", Box::new(Comrak::new())),
        ("ts raw", Box::new(TreeSitter::new())),
    ]
}

/// `--only a,b` on the command line, as an environment variable.
pub fn selected() -> impl Iterator<Item = &'static Candidate> {
    let only = std::env::var("SPIKE_ONLY").unwrap_or_default();
    CANDIDATES
        .iter()
        .filter(move |c| only.is_empty() || only.split(',').any(|name| name == c.0))
}
