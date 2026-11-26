//! The candidates. The library ones wait for approval of their crates.

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
];
