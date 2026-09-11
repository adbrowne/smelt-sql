//! The maintenance driver's own SQL must be dialect-plural.
//!
//! `smelt-logical` owns every *statement* a run executes, but the driver
//! still assembles a handful of read-side `SELECT`s itself — the changed-key
//! projection an observed-delta record is computed from, the succession
//! step's window predicate, the repair-key literal relation. Those are SQL
//! text, and until a live BigQuery run of `examples/github_activity` hit
//! them they were written in DuckDB's spelling:
//!
//! - `CAST(<key> AS VARCHAR)` in `changed_keys_select` — GoogleSQL has no
//!   `VARCHAR` at all (`Type not found: VARCHAR`), so **every** keyed model
//!   with a suppressed write failed its observed-delta record on BigQuery.
//! - `<col> >= DATE '<start>'` in the succession step's window predicate —
//!   DuckDB widens `DATE` to `TIMESTAMP` implicitly, GoogleSQL refuses (`No
//!   matching signature for operator >= for argument types: TIMESTAMP,
//!   DATE`).
//!
//! Both passed every offline gate in the tree, because no gate looked at the
//! driver's own text. This one does, in two legs:
//!
//! 1. **Behavioural** — build each of those fragments for BigQuery and
//!    assert the DuckDB-only token is gone and the GoogleSQL one is present
//!    (with the DuckDB rendering asserted too, so the test cannot pass by
//!    the function ignoring its dialect argument).
//! 2. **Structural** — scan production sources under
//!    `src/maintenance_driver/` for the DuckDB-only type names GoogleSQL
//!    rejects, so the next driver-authored fragment cannot reintroduce the
//!    class. A site that is genuinely DuckDB-only carries an inline
//!    `DIALECT-OK:` note naming why.

use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_runtime::maintenance_driver::{
    changed_keys_select, keyed_fold_changed_keys_select, repair_keys_literal_select,
    succession_window_predicate,
};

// ---------------------------------------------------------------- leg 1

#[test]
fn changed_keys_select_casts_to_the_dialects_own_string_type() {
    let compared = vec!["score".to_string()];
    let bq = changed_keys_select(
        "ds.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM ds.src_users",
        &compared,
        Some("event_date"),
        MaintenanceDialect::BigQuery,
    );
    assert!(
        !bq.contains("VARCHAR"),
        "GoogleSQL has no VARCHAR type; got: {bq}"
    );
    assert!(
        bq.contains("CAST(source.user_id AS STRING)")
            && bq.contains("CAST(source.event_date AS STRING)"),
        "BigQuery's key and partition projections must cast to STRING; got: {bq}"
    );

    // The DuckDB rendering is asserted too, so this pair cannot both pass by
    // the function ignoring the dialect it is handed.
    let duck = changed_keys_select(
        "main.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM main.src_users",
        &compared,
        Some("event_date"),
        MaintenanceDialect::DuckDb,
    );
    assert!(
        duck.contains("CAST(source.user_id AS VARCHAR)"),
        "DuckDB keeps its own VARCHAR spelling; got: {duck}"
    );
}

#[test]
fn keyed_fold_changed_keys_select_casts_to_the_dialects_own_string_type() {
    let compared = vec!["score".to_string()];
    let folds = vec![(
        "score".to_string(),
        "GREATEST(target.score, delta.score)".to_string(),
    )];
    // Two key columns: the composite `CONCAT(...)` branch is a separate
    // rendering path from the single-key one, and carried its own hardcode.
    let bq = keyed_fold_changed_keys_select(
        "ds.dim_scores",
        &["user_id".to_string(), "day".to_string()],
        "SELECT * FROM ds.src_scores",
        &compared,
        &folds,
        None,
        MaintenanceDialect::BigQuery,
    );
    assert!(
        !bq.contains("VARCHAR"),
        "GoogleSQL has no VARCHAR type; got: {bq}"
    );
    assert!(
        bq.contains("CAST(delta.user_id AS STRING)") && bq.contains("CAST(delta.day AS STRING)"),
        "every composite-key component casts to the dialect's string type; got: {bq}"
    );
}

#[test]
fn the_succession_window_predicate_uses_untyped_date_literals() {
    let p = succession_window_predicate("created_at", "2026-08-06", "2026-08-07");
    assert_eq!(
        p, "created_at >= '2026-08-06' AND created_at < '2026-08-07'",
        "a typed DATE literal here is a TIMESTAMP-column type error on GoogleSQL"
    );
    assert!(!p.contains("DATE '"), "got: {p}");
}

#[test]
fn the_empty_repair_key_relation_is_typed_per_dialect() {
    let bq = repair_keys_literal_select(&[], MaintenanceDialect::BigQuery);
    assert!(!bq.contains("VARCHAR"), "got: {bq}");
    assert!(bq.contains("CAST(NULL AS STRING)"), "got: {bq}");
    let duck = repair_keys_literal_select(&[], MaintenanceDialect::DuckDb);
    assert!(duck.contains("CAST(NULL AS VARCHAR)"), "got: {duck}");
}

// ---------------------------------------------------------------- leg 2

/// Type names DuckDB accepts and GoogleSQL rejects outright (`Type not
/// found: …`). `BIGINT`/`BOOLEAN`/`DECIMAL` are deliberately absent —
/// GoogleSQL accepts those as aliases for `INT64`/`BOOL`/`NUMERIC`, so they
/// are not part of this class.
const DUCKDB_ONLY_TYPE_NAMES: &[&str] = &["VARCHAR", "TEXT", "DOUBLE", "REAL", "FLOAT"];

/// An occurrence that is genuinely DuckDB-only carries this marker on the
/// same line or the line above, with a reason — the same discipline
/// `state_guard_census`'s `STATE-GUARD` annotation uses.
const WAIVER: &str = "DIALECT-OK:";

fn is_test_path(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "tests" || s == "tests.rs"
    })
}

fn rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") && !is_test_path(&path) {
            out.push(path);
        }
    }
}

/// A line is only interesting if it looks like emitted SQL rather than
/// prose: the token must be preceded by `AS ` or `::` (a cast) inside the
/// line. Doc comments and ordinary comments are skipped outright.
fn offending_casts(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let waived = line.contains(WAIVER)
            || (i > 0 && lines[i - 1].contains(WAIVER))
            || (i > 1 && lines[i - 2].contains(WAIVER));
        if waived {
            continue;
        }
        for ty in DUCKDB_ONLY_TYPE_NAMES {
            if line.contains(&format!("AS {ty}")) || line.contains(&format!("::{ty}")) {
                found.push((i + 1, (*line).to_string()));
                break;
            }
        }
    }
    found
}

#[test]
fn the_driver_authors_no_duckdb_only_type_cast() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maintenance_driver");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(files.len() > 5, "walk found nothing: {files:?}");

    let mut offences = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source");
        for (line_no, line) in offending_casts(&text) {
            offences.push(format!("{}:{}: {}", file.display(), line_no, line.trim()));
        }
    }
    assert!(
        offences.is_empty(),
        "the maintenance driver must not author a cast to a type GoogleSQL has no name for \
         (`Type not found: …`). Route the spelling through \
         `smelt_logical::maintenance::emit::probe_dialect_string_type(dialect)`, or mark the \
         site `{WAIVER} <reason>` if it is genuinely DuckDB-only:\n{}",
        offences.join("\n")
    );
}

/// The scanner must actually see a planted offence — otherwise the gate
/// above could pass because the walk, the comment filter or the cast
/// heuristic silently matched nothing.
#[test]
fn the_scanner_fails_closed_on_a_planted_cast() {
    let planted = "fn f() {\n    format!(\"SELECT CAST(x AS VARCHAR) AS k\")\n}\n";
    assert_eq!(offending_casts(planted).len(), 1, "planted cast missed");

    // …and the waiver really waives, on the line and on the line above.
    let waived_inline = "    format!(\"CAST(x AS VARCHAR)\") // DIALECT-OK: DuckDB-only path\n";
    assert!(offending_casts(waived_inline).is_empty());
    let waived_above = "    // DIALECT-OK: DuckDB-only path\n    format!(\"CAST(x AS VARCHAR)\")\n";
    assert!(offending_casts(waived_above).is_empty());

    // A prose mention is not an offence.
    assert!(offending_casts("/// joins each `CAST(c AS VARCHAR)` column\n").is_empty());
}

/// State-structure statement families that have more than one dialect's
/// renderer behind them. Each has a single `SqlDialect`-keyed dispatch
/// module in `smelt-state` (`ledger`, `observed_delta`, `tombstone`), and a
/// consumer must go through it — naming `ddl_duckdb::generate_…` directly is
/// unreachable-and-harmless only while the structure is DuckDB-only, and
/// becomes DuckDB SQL in a BigQuery job the moment the availability layer
/// flips that dialect's row on. That is exactly how `_smelt_ledger`'s
/// `CREATE TABLE … (model_name VARCHAR NOT NULL, …)` reached a live BigQuery
/// run from `execute/project/mod.rs` after phase 13 declared the
/// reconciliation ledger realisable there.
///
/// The fingerprint sidecar is deliberately absent: it is declared
/// single-dialect (`BackendCapabilities::supports_fingerprint_sidecar`, true
/// for DuckDB alone) and every consumer gates on that capability, so it has
/// no dispatch module to route through.
const DISPATCHED_STATEMENT_FAMILIES: &[&str] = &["ledger", "observed_delta", "tombstone"];

#[test]
fn a_multi_dialect_state_structure_is_never_reached_by_its_duckdb_builder() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(files.len() > 20, "walk found nothing: {}", files.len());

    let needles: Vec<String> = ["ddl_duckdb", "ddl_bigquery", "ddl_spark"]
        .iter()
        .flat_map(|m| {
            DISPATCHED_STATEMENT_FAMILIES
                .iter()
                .map(move |f| format!("{m}::generate_{f}"))
        })
        .collect();

    let mut offences = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in &needles {
                if line.contains(needle.as_str()) {
                    offences.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "a state structure with more than one dialect's renderer must be reached through its \
         `smelt_state` dispatch module (`ledger` / `observed_delta` / `tombstone`), never a \
         named per-dialect builder:\n{}",
        offences.join("\n")
    );
}

/// The same class, one layer down: `DATE '…'` typed literals in the
/// driver's own predicates. GoogleSQL will not compare a `TIMESTAMP` column
/// to a `DATE`, and the driver never knows the column's type here.
#[test]
fn the_driver_authors_no_typed_date_literal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maintenance_driver");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    let mut offences = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("DATE '") || line.contains("TIMESTAMP '") {
                offences.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "a typed date/timestamp literal in a driver-authored predicate is a cross-dialect trap \
         — DuckDB widens DATE to TIMESTAMP implicitly, GoogleSQL refuses. Emit an untyped \
         string literal instead:\n{}",
        offences.join("\n")
    );
}
