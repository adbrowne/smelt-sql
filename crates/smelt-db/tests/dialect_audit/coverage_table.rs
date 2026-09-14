//! The published table.

use crate::report;
use smelt_types::BuiltinRegistry;

const COVERAGE_DOC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/reference/dialect-coverage.md"
);

#[test]
fn the_coverage_table_matches_the_registry() {
    let rendered = report::render();
    let on_disk = std::fs::read_to_string(COVERAGE_DOC).unwrap_or_default();
    if std::env::var("SMELT_REGEN_DOCS").as_deref() == Ok("1") {
        if on_disk != rendered {
            std::fs::write(COVERAGE_DOC, &rendered).expect("write coverage doc");
        }
        return;
    }
    assert_eq!(
        on_disk, rendered,
        "docs/reference/dialect-coverage.md is stale. Regenerate with:\n  \
         SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \
         the_coverage_table_matches_the_registry"
    );
}

/// Totality on the published side. The table is the deliverable, and a gate
/// that only checked freshness would let an entry vanish from it silently.
#[test]
fn every_entry_and_dialect_appears_in_the_table() {
    let rendered = report::render();
    for name in BuiltinRegistry::names() {
        assert!(
            rendered.contains(&format!("| `{name}` |")),
            "{name} missing from the table"
        );
    }
    // Every dialect has a verification-tier row: the table's honesty depends
    // on saying which cells a live leg actually visits.
    for label in ["DuckDB", "Spark SQL", "BigQuery", "Trino"] {
        assert!(
            rendered.contains(&format!("| {label} |")),
            "{label} has no verification-tier row"
        );
    }
}
