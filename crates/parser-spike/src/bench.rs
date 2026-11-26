//! Criteria 1 and 2: cold parse and the tree update per keystroke.

use std::hint::black_box;
use std::time::Instant;

use crate::Rng;
use crate::backends::Candidate;
use crate::tree::{self, BlockParser, Ctx, Edit, Kind, Unit};

const COLD_RUNS: usize = 15;
const SITES: usize = 1000;

fn median_ms(mut run: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..COLD_RUNS)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

pub fn cold(candidate: &Candidate, text: &str) {
    let sequential = candidate.1().parse(text);
    let nodes: usize = sequential.iter().map(|u| 1 + u.children.len()).sum();
    // Building the tree and dropping it: both are part of a reparse.
    let single = median_ms(|| {
        black_box(candidate.1().parse(black_box(text)));
    });
    print!(
        "{:13} cold {single:7.2} ms ({} units, {nodes} blocks)",
        candidate.0,
        sequential.len()
    );
    for threads in [4, 8, 16] {
        let make = candidate.1;
        let parse = || tree::parse_threaded(text, threads, || Boxed(make()));
        let time = median_ms(|| {
            black_box(parse());
        });
        let same = if parse() == sequential {
            ""
        } else {
            " DIFFERS"
        };
        print!(" | {threads}t {time:6.2}{same}");
    }
    println!();
}

struct Boxed(Box<dyn BlockParser + Send>);

impl BlockParser for Boxed {
    fn parse_from(&mut self, text: &str, from: usize, ctx: Ctx, emit: &mut tree::Emit<'_>) {
        self.0.parse_from(text, from, ctx, emit);
    }
}

/// One scenario: keystrokes typed at a site and then undone.
#[derive(Clone, Copy)]
pub enum Typing {
    Character,
    Enter,
    Fence,
    FenceNeverClosed,
    FenceAutoClosed,
    FenceInAside,
    AsideCloser,
}

pub const TYPINGS: &[(&str, Typing)] = &[
    ("char in paragraph", Typing::Character),
    ("Enter in paragraph", Typing::Enter),
    ("``` bare, stays open", Typing::Fence),
    ("~~~ nothing closes it", Typing::FenceNeverClosed),
    ("``` auto-closed (K12)", Typing::FenceAutoClosed),
    ("``` bare, in aside", Typing::FenceInAside),
    ("typing </jade-aside>", Typing::AsideCloser),
];

impl Typing {
    fn keystrokes(self) -> &'static [&'static str] {
        match self {
            Typing::Character => &["e"],
            Typing::Enter => &["\n"],
            Typing::Fence | Typing::FenceInAside => &["`", "`", "`"],
            Typing::FenceAutoClosed => &["`", "`", "`"],
            Typing::FenceNeverClosed => &["~", "~", "~"],
            Typing::AsideCloser => &[
                "<", "/", "j", "a", "d", "e", "-", "a", "s", "i", "d", "e", ">",
            ],
        }
    }

    /// Places this can be typed: inside a paragraph, or on an empty line
    /// (made by inserting a break after a block).
    fn sites(self, text: &str, units: &[Unit]) -> Vec<usize> {
        let in_aside =
            |u: &&Unit| u.ctx == Ctx::Aside && matches!(u.kind, Kind::Table | Kind::Heading);
        match self {
            Typing::Character | Typing::Enter => units
                .iter()
                .filter(|u| u.kind == Kind::Paragraph)
                .map(|u| {
                    let mut at = u.start as usize + (u.start as usize * 31 % (u.len as usize + 1));
                    while !text.is_char_boundary(at) {
                        at -= 1;
                    }
                    at
                })
                .collect(),
            Typing::Fence | Typing::FenceNeverClosed | Typing::FenceAutoClosed => units
                .iter()
                .filter(|u| u.ctx == Ctx::Top && u.kind == Kind::Paragraph)
                .map(Unit::end)
                .collect(),
            Typing::FenceInAside | Typing::AsideCloser => {
                units.iter().filter(in_aside).map(Unit::end).collect()
            }
        }
    }

    fn needs_empty_line(self) -> bool {
        !matches!(self, Typing::Character | Typing::Enter)
    }
}

struct Session<'a> {
    parser: &'a mut dyn BlockParser,
    text: String,
    units: Vec<Unit>,
    fresh: Vec<Unit>,
}

impl Session<'_> {
    /// Replaces text and updates the tree; times only the tree update.
    fn edit(&mut self, at: usize, removed: usize, inserted: &str) -> (f64, usize) {
        self.text.replace_range(at..at + removed, inserted);
        let edit = Edit {
            at,
            removed,
            inserted: inserted.len(),
        };
        let start = Instant::now();
        let done = tree::update(
            self.parser,
            &mut self.units,
            &mut self.fresh,
            &self.text,
            edit,
        );
        (start.elapsed().as_secs_f64() * 1e3, done.reparsed_units)
    }
}

pub fn edits(candidate: &Candidate, text: &str, label: &str, typing: Typing) {
    let mut parser = candidate.1();
    let units = parser.parse(text);
    let sites = typing.sites(text, &units);
    if sites.is_empty() {
        return;
    }
    let pristine = units.clone();
    let mut session = Session {
        parser: parser.as_mut(),
        text: text.to_string(),
        units,
        fresh: Vec::with_capacity(1024),
    };
    let (mut times, mut reparsed) = (Vec::new(), Vec::new());
    let mut rng = Rng(42);
    // At least 1,000 timed keystrokes per scenario.
    while times.len() < SITES {
        let site = sites[rng.below(sites.len())];
        if typing.needs_empty_line() {
            session.edit(site, 0, "\n\n");
        }
        let start = site + if typing.needs_empty_line() { 2 } else { 0 };
        let mut at = start;
        let keys = typing.keystrokes();
        for (i, key) in keys.iter().enumerate() {
            // K12: the keystroke that would open a fence also closes it.
            let closes = matches!(typing, Typing::FenceAutoClosed) && i + 1 == keys.len();
            let inserted = if closes { "`\n\n```" } else { key };
            let (time, units) = session.edit(at, 0, inserted);
            at += key.len();
            times.push(time);
            reparsed.push(units);
        }
        let added = session.text.len() - text.len();
        session.edit(site, added, "");
    }
    let intact = session.units == pristine && session.text == text;
    times.sort_by(f64::total_cmp);
    reparsed.sort_unstable();
    let at = |q: usize| times[(times.len() - 1) * q / 100];
    println!(
        "{:13} {label:24} p50 {:8.4} ms  p99 {:8.4} ms  max {:8.3} ms  units reparsed p50 {} p99 {}  (n={}){}",
        candidate.0,
        at(50),
        at(99),
        times[times.len() - 1],
        reparsed[reparsed.len() / 2],
        reparsed[(reparsed.len() - 1) * 99 / 100],
        times.len(),
        if intact { "" } else { "  TREE NOT RESTORED" },
    );
}
