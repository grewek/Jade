//! `tests/acceptance/INDEX.md`, generated from the scenario files and checked
//! against them, so it cannot go stale.

use std::fmt::Write as _;

use super::format::{LeftOut, Scenario};

pub(super) fn render(scenarios: &[Scenario], left_out: &LeftOut) -> String {
    let mut sorted: Vec<&Scenario> = scenarios.iter().collect();
    sorted.sort_by_key(|s| (s.from_stage, &s.area, &s.id));
    let mut out = String::from(
        "# Acceptance catalog\n\n\
         Generated from the scenario files; do not edit. After changing one, run\n\
         `JADE_UPDATE_INDEX=1 cargo test -p jade-app acceptance_index -- --test-threads=1`.\n\n\
         | Scenario | SPEC rules | Stage | Source |\n\
         |---|---|---|---|\n",
    );
    for s in sorted {
        let spec = if s.spec.is_empty() {
            "–".to_owned()
        } else {
            s.spec.join(", ")
        };
        let _ = writeln!(
            out,
            "| [{id}]({area}/{id}.toml) | {spec} | {stage} | {source} |",
            id = s.id,
            area = s.area,
            stage = s.from_stage,
            source = s.source,
        );
    }
    out.push_str("\n## Still owed\n\n| Source | Stage | Scenario |\n|---|---|---|\n");
    for owed in &left_out.owed {
        let _ = writeln!(out, "| {} | {} | {} |", owed.source, owed.stage, owed.what);
    }
    out.push_str("\n## Fix commits without a scenario\n\n| Reason | Commits |\n|---|---|\n");
    for group in &left_out.without {
        let _ = writeln!(out, "| {} | {} |", group.reason, group.commits.join(", "));
    }
    out
}
