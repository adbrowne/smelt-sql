use std::fs;

use super::repo_root;

/// Claims `docs/specs/model_properties.md` §Known Divergences must not restate once the
/// composition walk actually closes them (`docs/outcomes/20260904-walk-migration-residue/
/// outcome.md` phase 7). Each entry is the exact substring the closed claim used; a stale
/// restatement of either fails this test.
const CLOSED_WALK_GAP_CLAIMS: &[&str] = &[
    "Only one maintenance-cell route consults a declared-RI closure today",
    "whole-SQL `OVER(` scan",
];

/// Extract the `## Known Divergences` section body (up to the next `## ` heading) from a
/// `model_properties.md`-shaped markdown document. `None` if the heading is missing, so a
/// caller can distinguish "found the section, it's clean" from "never looked".
fn known_divergences_section(markdown: &str) -> Option<&str> {
    let start = markdown.find("## Known Divergences")?;
    let after_heading = &markdown[start..];
    let body_start = after_heading.find('\n')? + 1;
    let body = &after_heading[body_start..];
    let end = body.find("\n## ").unwrap_or(body.len());
    Some(&body[..end])
}

fn find_closed_walk_gap_claim(section: &str) -> Option<&'static str> {
    CLOSED_WALK_GAP_CLAIMS
        .iter()
        .copied()
        .find(|phrase| section.contains(phrase))
}

/// Durable regression lock for phase 7's divergence-bullet deletions: `model_properties.md`
/// §Known Divergences must never again claim only one maintenance-cell route consults a
/// declared-RI closure (closed by phase 5) or that a whole-SQL `OVER(` scan still governs
/// cumulative classification (closed by phase 4).
#[test]
fn spec_divergences_do_not_claim_closed_walk_gaps() {
    let root = repo_root();
    let spec_path = root.join("docs/specs/model_properties.md");
    let text = fs::read_to_string(&spec_path).expect("read docs/specs/model_properties.md");
    let section = known_divergences_section(&text)
        .expect("model_properties.md has a '## Known Divergences' heading");
    let found = find_closed_walk_gap_claim(section);
    assert!(
        found.is_none(),
        "docs/specs/model_properties.md §Known Divergences restates a claim the \
         20260904-walk-migration-residue outcome already closed: {found:?}"
    );
}

/// Guards the gate above against silently passing because it failed to locate the section at
/// all (e.g. a future heading rename) rather than because the section is actually clean.
#[test]
fn spec_divergence_gate_detects_a_stale_claim() {
    let synthetic = "# Spec\n\n\
         ## Known Divergences\n\n\
         - Only one maintenance-cell route consults a declared-RI closure today, in fact.\n\n\
         ## Future Extensions\n\nsomething unrelated\n";
    let section =
        known_divergences_section(synthetic).expect("synthetic doc has the expected heading");
    assert_eq!(
        find_closed_walk_gap_claim(section),
        Some("Only one maintenance-cell route consults a declared-RI closure today"),
        "gate failed to detect a stale claim planted in a synthetic §Known Divergences body"
    );
}
