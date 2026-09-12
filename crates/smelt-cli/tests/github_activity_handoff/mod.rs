//! Drift gates over the criterion-8 findings handoff
//! (`docs/handoffs/2026-09-08-github-activity-findings.md`) — string-level
//! checks over a committed document, split out of `github_activity_oracle.rs`
//! so neither file grows into a token-cost hot spot. They share that suite's
//! [`DIVERGENCE_REGISTRY`](super::DIVERGENCE_REGISTRY), which is the whole
//! reason they live in this test binary rather than one of their own.

use super::DIVERGENCE_REGISTRY;

/// The criterion-8 findings handoff: its offline half banked by phase 15
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/15-plan.md`) and its
/// live-BigQuery half by phase 16 (`phases/16-plan.md`) — a cheap,
/// string-level drift gate over a committed doc, not a content check. Both
/// halves have landed; the tests below hold the document to saying so, to
/// naming the model and statement behind every live finding, and to stating
/// what the met criteria rest on rather than rounding them up.
const FINDINGS_HANDOFF: &str =
    include_str!("../../../../docs/handoffs/2026-09-08-github-activity-findings.md");

/// Relation names the handoff's "five registered divergences" table claims
/// are registered — parsed from its own markdown table rather than assumed,
/// so a renamed or retired `DIVERGENCE_REGISTRY` entry cannot leave a stale
/// row silently behind (test `findings_handoff_names_no_unknown_relation`).
///
/// Scoped to the `## The registered divergences` section (bounded by its
/// heading and the next `##`/`###` heading): the generic "any backtick-leading
/// table row" scan used to run over the whole document, which flagged phase
/// 9's unrelated `## Criterion 6` table (a test-name column, not a registered
/// relation) as a stale registered-divergence claim — see
/// `the_handoff_scan_is_scoped_to_the_divergence_section`.
fn handoff_claimed_relations() -> Vec<String> {
    claimed_relations_in(FINDINGS_HANDOFF)
}

/// The pure scan behind [`handoff_claimed_relations`], taking the document
/// text as a parameter so the scoping behaviour itself is testable against a
/// synthetic document rather than only the one committed handoff.
fn claimed_relations_in(text: &str) -> Vec<String> {
    const SECTION_HEADING: &str = "## The registered divergences";
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == SECTION_HEADING) else {
        return Vec::new();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            let l = l.trim();
            l.starts_with("## ") || l.starts_with("### ")
        })
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    lines[start + 1..end]
        .iter()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("| `") {
                return None;
            }
            let rest = &line[3..];
            let end = rest.find('`')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

/// Test 6 (phase 15, accept direction): every `DIVERGENCE_REGISTRY` entry's
/// relation name appears verbatim in the findings handoff.
#[test]
fn every_registry_entry_is_named_in_the_findings_handoff() {
    for entry in DIVERGENCE_REGISTRY {
        assert!(
            FINDINGS_HANDOFF.contains(entry.relation),
            "docs/handoffs/2026-09-08-github-activity-findings.md does not name registry \
             relation `{}` — every DIVERGENCE_REGISTRY entry must be traceable in the \
             findings handoff",
            entry.relation
        );
    }
}

/// Test 7 (phase 15, reverse direction): every relation the handoff's
/// divergence table claims is registered actually resolves to a
/// `DIVERGENCE_REGISTRY` entry, so a renamed or retired entry cannot leave a
/// stale row in the document. `DIVERGENCE_REGISTRY` is now empty (phase 7 of
/// `docs/outcomes/20260906-bigquery-correctness` fixed the fourth and final
/// root cause rather than registering it), so the handoff's own divergence
/// table is expected to claim nothing either — the reverse-direction check
/// becomes "the doc claims no stale registered relation."
#[test]
fn findings_handoff_names_no_unknown_relation() {
    let claimed = handoff_claimed_relations();
    if DIVERGENCE_REGISTRY.is_empty() {
        assert!(
            claimed.is_empty(),
            "DIVERGENCE_REGISTRY is empty, but the findings handoff's divergence table still \
             claims relation(s) {claimed:?} as registered — stale table row"
        );
        return;
    }
    for relation in &claimed {
        assert!(
            DIVERGENCE_REGISTRY.iter().any(|e| e.relation == relation),
            "findings handoff names relation `{relation}` as registered, but no \
             DIVERGENCE_REGISTRY entry with that name exists — stale or renamed entry"
        );
    }
}

/// Phase 10 test 4: a backtick-leading table row in a section *other* than
/// `## The registered divergences` (e.g. phase 9's `## Criterion 6` test-name
/// table) is not read as a registered-divergence claim. RED before
/// `handoff_claimed_relations()` was scoped to that section.
#[test]
fn the_handoff_scan_is_scoped_to_the_divergence_section() {
    let synthetic = "\
## The registered divergences

Nothing registered here.

## Criterion 6 — the two known live conformance failures

| Test: `diamond_propagation_suffices` | fixed |
| Test: `composed_keyed_pool_upholds_equivalence` | fixed |

## References
";
    let claimed = claimed_relations_in(synthetic);
    assert!(
        claimed.is_empty(),
        "expected no claimed relations from a table outside the divergence section, got \
         {claimed:?}"
    );
}

/// Phase 10 test 5: non-vacuity for test 4 — a fabricated backtick-leading
/// row *inside* the divergence section is still collected, so
/// `findings_handoff_names_no_unknown_relation` keeps its teeth.
#[test]
fn the_handoff_scan_still_catches_a_stale_claim_in_its_own_section() {
    let synthetic = "\
## The registered divergences

| `some_stale_relation` | still claimed |

## Criterion 6 — the two known live conformance failures

Unrelated section.
";
    let claimed = claimed_relations_in(synthetic);
    assert_eq!(
        claimed,
        vec!["some_stale_relation".to_string()],
        "expected the in-section row to still be collected, got {claimed:?}"
    );
}

/// The live-BigQuery findings table's rows, scoped to the
/// `## Live-BigQuery findings` section — parsed from the document's own
/// markdown rather than assumed, so a row that stops naming the model or the
/// statement that provoked it fails rather than reading as prose.
///
/// Each returned entry is the row's five cells:
/// `| finding | provoking model | provoking statement | classification | owner |`.
/// The header row and the `---` separator are skipped; a table in any other
/// section is not collected (see `live_findings_scan_is_scoped_and_non_vacuous`).
fn live_findings_in(text: &str) -> Vec<Vec<String>> {
    const SECTION_HEADING: &str = "## Live-BigQuery findings";
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == SECTION_HEADING) else {
        return Vec::new();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            let l = l.trim();
            l.starts_with("## ") || l.starts_with("### ")
        })
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    lines[start + 1..end]
        .iter()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('|') {
                return None;
            }
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect();
            if cells.len() != 5 {
                return None;
            }
            if cells[0].eq_ignore_ascii_case("finding") {
                return None;
            }
            if cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
            {
                return None;
            }
            Some(cells)
        })
        .collect()
}

/// Phase 16 test 1 (replaces `findings_handoff_declares_its_interim_status`):
/// the handoff no longer declares itself interim — both halves have landed,
/// and the live addendum is dated so a reader can tell how old the live
/// evidence is.
#[test]
fn findings_handoff_declares_both_halves_landed() {
    assert!(
        !FINDINGS_HANDOFF.contains("DuckDB half only"),
        "findings handoff still declares itself the DuckDB half only — phase 16 banked the \
         live-BigQuery half, so the interim framing must be gone"
    );
    assert!(
        FINDINGS_HANDOFF.contains("## The live BigQuery half"),
        "findings handoff must carry the live-BigQuery half's own section"
    );
    assert!(
        FINDINGS_HANDOFF.contains("2026-09-12"),
        "findings handoff must date the live addendum (2026-09-12)"
    );
}

/// Phase 16 test 2: every row of the live-findings table names the model and
/// the statement that provoked it — criterion 8's own wording ("each with the
/// model and statement that provoked it"), checked rather than trusted.
#[test]
fn live_findings_each_name_a_provoking_model_and_statement() {
    let rows = live_findings_in(FINDINGS_HANDOFF);
    assert!(
        rows.len() >= 7,
        "expected the live-findings table to carry at least the seven findings phase 16 \
         harvested, got {}",
        rows.len()
    );
    for row in &rows {
        let finding = &row[0];
        for (idx, label) in [(1usize, "provoking model"), (2, "provoking statement")] {
            let cell = row[idx].trim();
            assert!(
                !cell.is_empty() && cell != "—" && cell != "-" && cell != "n/a",
                "live finding `{finding}` has an empty {label} cell — every row must name \
                 what provoked it"
            );
        }
        assert!(
            !row[3].trim().is_empty(),
            "live finding `{finding}` has no classification"
        );
        assert!(
            !row[4].trim().is_empty(),
            "live finding `{finding}` has no owner"
        );
    }
}

/// Phase 16 test 3: the live-findings scan is scoped to its own section, and
/// is not vacuous — a five-cell row outside the section is not collected, one
/// inside it is.
#[test]
fn live_findings_scan_is_scoped_and_non_vacuous() {
    let outside = "\
## Live-BigQuery findings

Nothing tabulated here.

## Operational notes for the next live run

| something | else | entirely | four | five |
";
    assert!(
        live_findings_in(outside).is_empty(),
        "expected a five-cell row outside the live-findings section not to be collected"
    );

    let inside = "\
## Live-BigQuery findings

| finding | provoking model | provoking statement | classification | owner |
|---|---|---|---|---|
| a finding | `a.model` | `SELECT 1` | defect | someone |

## References
";
    let rows = live_findings_in(inside);
    assert_eq!(rows.len(), 1, "expected the in-section row to be collected");
    assert_eq!(rows[0][1], "`a.model`");
    assert_eq!(rows[0][2], "`SELECT 1`");
}

/// Phase 16 test 4: the handoff records what the met criteria actually rest
/// on, so a reader cannot mistake "criteria 5, 6 and 7 are met" for
/// "everything is covered". The BigQuery half is 14 of 16 models (two refused
/// at compile time on GoogleSQL) at 7 of 30 oracle checkpoints.
#[test]
fn findings_handoff_records_what_the_criteria_rest_on() {
    assert!(
        FINDINGS_HANDOFF.contains("14 of 16"),
        "findings handoff must state the live half's model coverage (14 of 16)"
    );
    assert!(
        FINDINGS_HANDOFF.contains("7 of 30"),
        "findings handoff must state the live half's checkpoint coverage (7 of 30)"
    );
}
