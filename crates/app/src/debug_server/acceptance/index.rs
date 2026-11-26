//! `tests/acceptance/INDEX.md`, generated from the scenario files and checked
//! against them, so it cannot go stale.

use std::fmt::Write as _;

use super::format::Scenario;

pub(super) fn render(scenarios: &[Scenario]) -> String {
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
    out
}
