//! Drift gate tying `docs/specs/state.md`'s "Which dialects realise which
//! structure" table to `realisable_state_structures` (`docs/outcomes/
//! 20260913-trino-ledger/outcome.md` phase 2), so the spec's per-dialect
//! claim and the plan layer's actual claim cannot silently diverge — a new
//! `SqlDialect` variant with no spec column, or a spec cell that disagrees
//! with `realisable_state_structures`, fails here rather than being
//! discovered later as a downgrade that should not have happened (or did
//! not happen when it should have).

use std::fs;
use std::path::PathBuf;

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn state_spec() -> String {
    let path = repo_root().join("docs/specs/state.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn multi_backend_spec() -> String {
    let path = repo_root().join("docs/specs/multi_backend.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The realisability table's section, from its heading to the next `####`.
fn realisability_section(spec: &str) -> &str {
    let start = spec
        .find("#### Which dialects realise which structure")
        .expect("docs/specs/state.md has no §\"Which dialects realise which structure\"");
    let rest = &spec[start..];
    let end = rest[1..].find("\n#### ").map(|i| i + 1).unwrap_or_else(|| {
        rest[1..]
            .find("\n### ")
            .map(|i| i + 1)
            .unwrap_or(rest.len())
    });
    &rest[..end]
}

fn dialect_name_in_header(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::DuckDB => "DuckDB",
        SqlDialect::BigQuery => "BigQuery",
        SqlDialect::SparkSQL => "Spark",
        SqlDialect::Trino => "Trino",
    }
}

fn structure_row_label(structure: StateStructure) -> &'static str {
    match structure {
        StateStructure::MergeLedger => "Transactional merge ledger",
        StateStructure::ReconciliationLedger => "Reconciliation ledger",
        StateStructure::ObservedOutputDeltas => "Observed output deltas",
        StateStructure::FingerprintSidecar => "Fingerprint sidecar",
        StateStructure::TombstoneLedger => "Tombstone ledger",
    }
}

const ALL_DIALECTS: [SqlDialect; 4] = [
    SqlDialect::DuckDB,
    SqlDialect::SparkSQL,
    SqlDialect::BigQuery,
    SqlDialect::Trino,
];

const ALL_STRUCTURES: [StateStructure; 5] = [
    StateStructure::MergeLedger,
    StateStructure::ReconciliationLedger,
    StateStructure::ObservedOutputDeltas,
    StateStructure::FingerprintSidecar,
    StateStructure::TombstoneLedger,
];

/// Parses the `| Structure | DuckDB | ... |` table header and every data row
/// in the realisability section, returning `(header_cells, data_rows)`.
fn parse_table(section: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let header_start = section
        .find("| Structure |")
        .expect("realisability section has no `| Structure |` table header");
    let mut lines = section[header_start..].lines();
    let header_line = lines.next().expect("table header line missing");
    let header: Vec<String> = header_line
        .split('|')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();

    let mut rows = Vec::new();
    for line in lines {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('|') {
            break;
        }
        if trimmed.contains("---") {
            continue;
        }
        let cells: Vec<String> = line
            .split('|')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        rows.push(cells);
    }
    (header, rows)
}

/// Every `SqlDialect` must have a column, and every cell's yes/no must agree
/// with `realisable_state_structures(dialect)`.
#[test]
fn state_md_table_agrees_with_realisable_state_structures() {
    let spec = state_spec();
    let section = realisability_section(&spec);
    let (header, rows) = parse_table(section);

    for dialect in ALL_DIALECTS {
        let name = dialect_name_in_header(dialect);
        let col_idx = header
            .iter()
            .position(|c| c.contains(name))
            .unwrap_or_else(|| {
                panic!(
                    "docs/specs/state.md's realisability table has no column for {dialect:?} \
                     (looked for header cell containing {name:?}); header was {header:?}"
                )
            });

        let claimed: Vec<StateStructure> = realisable_state_structures(dialect);

        for structure in ALL_STRUCTURES {
            let label = structure_row_label(structure);
            let row = rows
                .iter()
                .find(|r| r.first().is_some_and(|s| s.contains(label)))
                .unwrap_or_else(|| {
                    panic!("docs/specs/state.md's realisability table has no row for {label:?}")
                });
            let cell = row.get(col_idx).unwrap_or_else(|| {
                panic!(
                    "docs/specs/state.md's realisability table row {label:?} has no cell for \
                     {dialect:?} (column {col_idx})"
                )
            });
            let spec_says_yes = cell.trim() == "yes";
            let code_says_yes = claimed.contains(&structure);
            assert_eq!(
                spec_says_yes, code_says_yes,
                "docs/specs/state.md claims {dialect:?}/{structure:?} = {cell:?} but \
                 realisable_state_structures says {code_says_yes} — the spec table and the \
                 plan layer disagree"
            );
        }
    }
}

#[test]
fn trino_absence_is_stated_as_permanent_with_the_measured_reason() {
    let spec = state_spec();
    let section = realisability_section(&spec);

    assert!(
        section.contains("Trino"),
        "docs/specs/state.md's realisability section does not mention Trino"
    );
    assert!(
        section.contains("permanent"),
        "docs/specs/state.md's realisability section does not state Trino's absence is \
         permanent"
    );
    assert!(
        section.to_lowercase().contains("autocommit"),
        "docs/specs/state.md's realisability section does not cite the measured autocommit \
         reason for Trino's absence"
    );
    assert!(
        section.contains("01-summary.md") || section.contains("trino-ledger"),
        "docs/specs/state.md's realisability section does not cite the phase 1 measurement \
         backing Trino's absence"
    );
}

#[test]
fn multi_backend_states_the_trino_state_posture() {
    let spec = multi_backend_spec();
    let start = spec
        .find("### Incremental & schema evolution per backend")
        .expect(
            "docs/specs/multi_backend.md has no §\"Incremental & schema evolution per backend\"",
        );
    let rest = &spec[start..];
    let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    let section = &rest[..end];

    assert!(
        section.contains("trino") || section.contains("Trino"),
        "docs/specs/multi_backend.md §\"Incremental & schema evolution per backend\" does not \
         name the trino target"
    );
    assert!(
        section.contains("MaintenanceStateDowngraded"),
        "docs/specs/multi_backend.md §\"Incremental & schema evolution per backend\" does not \
         name MaintenanceStateDowngraded as the consequence for the trino target"
    );
}

#[test]
fn no_spec_text_calls_trino_maintenance_not_yet_reachable() {
    let spec = multi_backend_spec();
    assert!(
        !spec.contains("not yet reachable"),
        "docs/specs/multi_backend.md still describes Trino maintenance techniques as \"not yet \
         reachable\" — phase 1's measurement settled this as a permanent absence, not pending \
         work"
    );
}
