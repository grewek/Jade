//! What the fuzzer found. The same fault shows up in many steps and runs with
//! other numbers in its text, so findings are grouped by their text without
//! the numbers and reported once, with where each was first seen.

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// The app panicked or ended by itself.
    Crash,
    /// The app lives but stopped answering.
    Hang,
    /// What the invariants query reported.
    Invariant,
    /// A frame over its budget.
    SlowFrame,
}

impl Kind {
    /// Whether a run cannot go on, and a CI run has to fail.
    pub(super) fn ends_the_app(self) -> bool {
        matches!(self, Kind::Crash | Kind::Hang)
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Crash => "crash",
            Kind::Hang => "hang",
            Kind::Invariant => "invariant",
            Kind::SlowFrame => "slow frame",
        }
    }
}

/// Where a finding was first seen.
#[derive(Debug, Clone)]
pub(super) struct Seen {
    pub seed: u64,
    /// The step after which it showed; 0 is opening the note.
    pub step: u32,
    /// The last steps up to that one, oldest first.
    pub trail: Vec<String>,
}

#[derive(Debug)]
pub(super) struct Finding {
    pub kind: Kind,
    pub text: String,
    pub first: Seen,
    pub times: u32,
}

/// Most different findings kept; further ones are only counted.
const MAX_FINDINGS: usize = 100;

#[derive(Debug, Default)]
pub(super) struct Findings {
    pub list: Vec<Finding>,
    pub dropped: u32,
}

/// `text` with every run of digits replaced by `#`.
fn signature(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if !c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('#') {
            out.push('#');
        }
    }
    out
}

impl Findings {
    /// Adds a finding; `true` if it is a new one.
    pub(super) fn add(&mut self, kind: Kind, text: &str, seen: impl FnOnce() -> Seen) -> bool {
        let same = signature(text);
        let known = self
            .list
            .iter_mut()
            .find(|f| f.kind == kind && signature(&f.text) == same);
        if let Some(finding) = known {
            finding.times += 1;
            return false;
        }
        if self.list.len() >= MAX_FINDINGS {
            self.dropped += 1;
            return false;
        }
        self.list.push(Finding {
            kind,
            text: text.to_owned(),
            first: seen(),
            times: 1,
        });
        true
    }

    /// Whether the run counts as failed. `crashes_only` is how CI judges:
    /// everything else is reported only.
    pub(super) fn fails(&self, crashes_only: bool) -> bool {
        self.list
            .iter()
            .any(|f| !crashes_only || f.kind.ends_the_app())
    }

    pub(super) fn report(&self) -> String {
        let mut out = format!("{} different findings", self.list.len());
        if self.dropped > 0 {
            let _ = write!(out, " ({} more were not kept)", self.dropped);
        }
        for f in &self.list {
            let _ = write!(
                out,
                "\n\n[{}] {}\n  seen {} times, first after step {} of seed {}\n  \
                 repeat: jade-debug fuzz --seed {} --runs 1 --steps {}",
                f.kind.label(),
                f.text.replace('\n', "\n    "),
                f.times,
                f.first.step,
                f.first.seed,
                f.first.seed,
                f.first.step.max(1),
            );
            for step in &f.first.trail {
                let _ = write!(out, "\n    {step}");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(seed: u64, step: u32) -> Seen {
        Seen {
            seed,
            step,
            trail: vec!["key up".into()],
        }
    }

    #[test]
    fn findings_that_differ_only_in_numbers_are_one() {
        let mut found = Findings::default();
        assert!(found.add(Kind::Invariant, "line 3 is at y=1.5", || seen(1, 2)));
        assert!(!found.add(Kind::Invariant, "line 41 is at y=20.0", || seen(1, 9)));
        assert!(found.add(Kind::Invariant, "word counts differ", || seen(2, 1)));
        assert_eq!((found.list.len(), found.list[0].times), (2, 2));
        let report = found.report();
        assert!(
            report.contains("seen 2 times, first after step 2 of seed 1"),
            "{report}"
        );
        assert!(
            report.contains("--seed 2 --runs 1 --steps 1\n    key up"),
            "{report}"
        );
    }

    #[test]
    fn only_a_crash_or_a_hang_fails_a_crashes_only_run() {
        let mut found = Findings::default();
        assert!(!found.fails(false));
        found.add(Kind::SlowFrame, "slow", || seen(1, 1));
        found.add(Kind::Invariant, "wrong", || seen(1, 1));
        assert!(found.fails(false) && !found.fails(true));
        found.add(Kind::Hang, "no reply", || seen(1, 1));
        assert!(found.fails(true));
    }
}
