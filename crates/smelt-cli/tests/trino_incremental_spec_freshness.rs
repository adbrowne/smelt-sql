//! Standing drift gate for Trino's maintenance surface
//! (`docs/outcomes/20260913-trino-incremental/outcome.md` phase 2): the spec
//! is written from what phase 1 measured against the live tier and what
//! `smelt_logical::maintenance::availability` derives, and these checks keep
//! the spec's own claims honest as later phases land real code.

use std::fs;
use std::path::PathBuf;

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::realisable_state_structures;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read_spec(name: &str) -> String {
    fs::read_to_string(repo_root().join("docs/specs").join(name)).unwrap()
}

fn section(text: &str, heading: &str) -> String {
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("multi_backend.md has no {heading} section"));
    let next = text[start + heading.len()..]
        .find("\n## ")
        .map(|i| start + heading.len() + i)
        .unwrap_or(text.len());
    text[start..next].to_string()
}

/// §"Whole-row MERGE" must name Trino as its own spelling family: both arms
/// rendered column-by-column, all three star/`ROW` shorthands refused, and
/// the `output_columns` + empty-projection-refusal machinery named for it.
#[test]
fn whole_row_merge_states_trinos_named_column_form() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Whole-row MERGE");

    assert!(
        sect.contains("Trino"),
        "§\"Whole-row MERGE\" does not mention Trino"
    );
    for shorthand in ["SET *", "INSERT *", "INSERT ROW"] {
        assert!(
            sect.contains(shorthand),
            "§\"Whole-row MERGE\" does not name `{shorthand}` as refused on Trino"
        );
    }
    assert!(
        sect.contains("output_columns"),
        "§\"Whole-row MERGE\" does not tie Trino's column-by-column form to `output_columns`"
    );
}

/// §"Column-scoped merge and conditional-write capabilities" must state that
/// Trino is now the second `false` backend for
/// `supports_merge_not_matched_by_source`, over a non-atomic
/// `TargetSchema`-resident staged group.
#[test]
fn conditional_write_section_states_trinos_absent_not_matched_by_source() {
    let text = read_spec("multi_backend.md");
    let sect = section(
        &text,
        "### Column-scoped merge and conditional-write capabilities",
    );

    assert!(
        sect.contains("Trino") && sect.contains("supports_merge_not_matched_by_source"),
        "the section does not state Trino's `supports_merge_not_matched_by_source` verdict"
    );
    assert!(
        sect.contains("TargetSchema") && sect.contains("staged_relation_group_is_atomic"),
        "the section does not tie Trino's departed-row delete to the non-atomic TargetSchema group"
    );
}

/// The same section must list the four clause forms phase 1 measured
/// accepted, since the merge-less conditional write is built from them.
#[test]
fn conditional_write_section_lists_the_measured_accepted_clause_forms() {
    let text = read_spec("multi_backend.md");
    let sect = section(
        &text,
        "### Column-scoped merge and conditional-write capabilities",
    );

    for form in [
        "WHEN MATCHED THEN DELETE",
        "WHEN MATCHED AND",
        "first-match-wins",
        "USING (SELECT",
    ] {
        assert!(
            sect.contains(form),
            "the section does not list the measured-accepted clause form `{form}`"
        );
    }
}

/// §"Incremental & schema evolution per backend" must state a landing state
/// for every `Technique` variant on Trino.
#[test]
fn incremental_section_states_a_landing_state_for_every_technique() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Incremental & schema evolution per backend");

    for technique in [
        "DeleteInsert",
        "PerGroupRecompute",
        "KeyedFold",
        "ColumnScopedMerge",
        "InPlaceUpdate",
        "SuccessionPatch",
    ] {
        assert!(
            sect.contains(technique),
            "§\"Incremental & schema evolution per backend\" does not name `{technique}`"
        );
    }
}

/// The spec's per-technique verdicts must agree with the pure availability
/// functions: `realisable_state_structures(Trino)` is empty (so every
/// structure-requiring technique is unrealisable there), and
/// `required_state_structure`'s own mapping (read from source, since building
/// a `PlanCell` needs no public test constructor and this phase is spec-only)
/// assigns a structure to every technique but `DeleteInsert`.
#[test]
fn spec_downgrade_table_matches_the_pure_availability_functions() {
    assert!(
        realisable_state_structures(SqlDialect::Trino).is_empty(),
        "realisable_state_structures(Trino) is no longer empty; the spec's fully-degraded \
         framing needs revisiting before this test is updated"
    );

    let source = fs::read_to_string(
        repo_root().join("crates/smelt-logical/src/maintenance/availability/state_structure.rs"),
    )
    .unwrap();
    let mapping_start = source
        .find("pub fn required_state_structure")
        .expect("state_structure.rs has no required_state_structure fn");
    let mapping = &source[mapping_start..];

    assert!(
        mapping.contains("Technique::KeyedFold => match cell.fold_grade")
            && mapping.contains("Some(StateStructure::ReconciliationLedger)"),
        "required_state_structure no longer maps an additive-graded (or undetermined-grade) \
         KeyedFold cell to the reconciliation ledger (`docs/specs/state.md` §\"The degradation \
         contract\" step 2 — the requirement is grade-dependent since phase 3g)"
    );
    assert!(
        mapping.contains("Technique::ColumnScopedMerge | Technique::InPlaceUpdate => {")
            || mapping.contains("Technique::ColumnScopedMerge | Technique::InPlaceUpdate =>"),
        "required_state_structure no longer maps ColumnScopedMerge/InPlaceUpdate to a required \
         structure"
    );
    assert!(
        mapping.contains("Technique::SuccessionPatch => Some("),
        "required_state_structure no longer maps SuccessionPatch to a required structure"
    );
    assert!(
        mapping.contains("Technique::DeleteInsert => None"),
        "required_state_structure no longer maps DeleteInsert to no required structure"
    );
    assert!(
        mapping.contains("KeyDiscovery::UpstreamKeyed | KeyDiscovery::DownstreamGrainOverUpstream")
            && mapping.contains("Some(StateStructure::FingerprintSidecar)"),
        "required_state_structure no longer maps key-addressed PerGroupRecompute to the \
         fingerprint sidecar"
    );

    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Incremental & schema evolution per backend");
    assert!(
        sect.contains("reachable") && (sect.contains("downgrad") || sect.contains("refus")),
        "the section does not distinguish reachable techniques from downgraded/refused ones"
    );
}

/// Phase 6c (`docs/outcomes/20260913-trino-incremental/phases/06c-plan.md`):
/// the `PerGroupRecompute`, no-`key_scope` row is split into a
/// repair-admitted row (downgraded) and a downgrade-reached row (reachable),
/// and `state.md`'s degradation-contract sentence states the repair-admitted
/// requirement rather than the stale "requires nothing" claim.
#[test]
fn per_group_recompute_no_key_scope_row_is_split_by_repair_admission() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Incremental & schema evolution per backend");

    assert!(
        sect.contains("repair-admitted") && sect.contains("downgrade-reached"),
        "§\"Incremental & schema evolution per backend\" does not distinguish a \
         repair-admitted PerGroupRecompute cell from a downgrade-reached one: {sect}"
    );

    let state_text = read_spec("state.md");
    let sect = section(&state_text, "### The degradation contract");
    assert!(
        sect.contains("repair-admitted") && sect.contains("fingerprint sidecar"),
        "state.md §\"The degradation contract\" does not state the repair-admitted \
         PerGroupRecompute cell's fingerprint-sidecar requirement"
    );
    assert!(
        sect.contains("downgrade-reached"),
        "state.md §\"The degradation contract\" no longer names the downgrade-reached \
         keyless, clamp-less cell that requires nothing"
    );
    assert!(
        sect.contains("clears the cell's now-meaningless")
            || sect.contains("clears the now-meaningless"),
        "state.md §\"The degradation contract\" no longer documents that a repair-admitted \
         cell's downgrade clears its ScanClamp"
    );
}

/// §"Generative equivalence coverage" must name Trino's `ConformanceTarget`
/// arm and its gated-tier test command (phase 8 —
/// `docs/outcomes/20260913-trino-incremental/phases/08-plan.md`), alongside
/// Spark's and BigQuery's.
#[test]
fn generative_equivalence_coverage_names_trino() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "**Generative equivalence coverage.**");

    assert!(
        sect.contains("Trino"),
        "§\"Generative equivalence coverage\" does not mention Trino"
    );
    assert!(
        sect.contains("maintenance_conformance_trino"),
        "§\"Generative equivalence coverage\" does not name the Trino test binary"
    );
    assert!(
        sect.contains("oracle relation") || sect.contains("temporary view"),
        "§\"Generative equivalence coverage\" does not state why Trino supplies its own oracle \
         relation"
    );
}

/// Trino introduces no new diagnostic code: the section names only the three
/// existing codes, and each exists in the diagnostics catalogue.
#[test]
fn no_trino_specific_diagnostic_code_is_introduced() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Incremental & schema evolution per backend");

    for code in [
        "MaintenanceStateDowngraded",
        "UnsupportedOnBackend",
        "DeclaredContractRequiresState",
    ] {
        assert!(
            sect.contains(code),
            "§\"Incremental & schema evolution per backend\" does not name `{code}`"
        );
    }
    assert!(
        !sect.contains("Trino") || !sect.to_lowercase().contains("trino diagnostic"),
        "the section appears to introduce a Trino-specific diagnostic vocabulary"
    );

    let diagnostics = read_spec("diagnostics.md");
    for code in [
        "MaintenanceStateDowngraded",
        "UnsupportedOnBackend",
        "DeclaredContractRequiresState",
    ] {
        assert!(
            diagnostics.contains(&format!("`{code}`")),
            "docs/specs/diagnostics.md has no catalogue entry for `{code}`"
        );
    }
}
