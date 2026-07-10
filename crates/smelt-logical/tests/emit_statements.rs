//! Correctness oracle for `smelt_logical::maintenance::emit`
//! (`docs/specs/maintenance_plan.md` §"Statement emission (single owner)"):
//! the emitters are the *single author* of every maintenance statement a
//! run executes. This file asserts each emitter's output shape directly
//! (byte-parity against production text is asserted from the execution side
//! in `crates/smelt-runtime/tests/statement_parity.rs`).

use smelt_logical::maintenance::emit::{
    emit_create_table_as, emit_delete_insert, emit_keyed_fold, MaintenanceDialect, Region,
};

#[test]
fn delete_insert_group_is_transactional_and_matches_production_shape() {
    let region = Region {
        start: "'2026-01-01'".to_string(),
        end: "'2026-01-08'".to_string(),
    };
    let body = "SELECT event_id, user_id, event_date FROM events \
                WHERE event_date >= '2026-01-01' AND event_date < '2026-01-08'";

    let group = emit_delete_insert(
        "main.clickstream",
        "event_date",
        &region,
        body,
        MaintenanceDialect::DuckDb,
    );

    assert!(
        group.transactional,
        "a paired region DELETE+INSERT must be one backend transaction \
         (a failed INSERT must roll back its DELETE)"
    );
    assert_eq!(group.statements.len(), 2);

    assert_eq!(
        group.statements[0].sql,
        "DELETE FROM main.clickstream WHERE event_date >= '2026-01-01' AND event_date < '2026-01-08'"
    );
    // The INSERT carries no redundant outer WHERE — `body` already carries
    // the output clamp (`docs/specs/model_transforms.md` §"the two
    // clamps"); the emitter must not re-wrap it in a second filter.
    assert_eq!(
        group.statements[1].sql,
        format!("INSERT INTO main.clickstream {body}")
    );
}

#[test]
fn delete_insert_escapes_quoted_literal_region_boundaries() {
    // A region boundary carrying a literal quote (e.g. from an untrusted
    // partition value) must come pre-escaped by the caller — the emitter
    // performs no escaping of its own, it only assembles the predicate. This
    // test documents that contract: the caller (the runtime's `PartitionRange`
    // → `Region` conversion) is the one responsible for `''`-escaping, and
    // an already-escaped boundary flows through unchanged.
    let region = Region {
        start: "'O''Brien'".to_string(),
        end: "'Z'".to_string(),
    };
    let group = emit_delete_insert(
        "main.t",
        "name",
        &region,
        "SELECT * FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements[0].sql,
        "DELETE FROM main.t WHERE name >= 'O''Brien' AND name < 'Z'"
    );
}

#[test]
fn delete_insert_dialect_invariant_shape() {
    // The region DELETE+INSERT family shares identical grammar across
    // DuckDB and Spark — no dialect-keyed variant needed for this family
    // (unlike the keyed-fold / column-scoped MERGE families, later phases).
    let region = Region {
        start: "'2026-01-01'".to_string(),
        end: "'2026-01-02'".to_string(),
    };
    let duckdb = emit_delete_insert("t", "d", &region, "SELECT 1", MaintenanceDialect::DuckDb);
    let spark = emit_delete_insert("t", "d", &region, "SELECT 1", MaintenanceDialect::Spark);
    assert_eq!(duckdb, spark);
}

/// The keyed-fold `MERGE` production actually executes for `refresh: keyed`
/// (`crate::cumulative::build_cumulative_merge_sql`'s pre-phase text):
/// combiner-aware `UPDATE SET`, `INSERT *` (no explicit column list — the
/// caller's `delta_select` projects exactly the target row's columns, so a
/// column-list `INSERT` would be a second, redundant restatement of what
/// the SELECT already guarantees).
#[test]
fn keyed_fold_renders_combiners_and_insert_star() {
    let group = emit_keyed_fold(
        "main.device_user_edges",
        &["device_id".to_string(), "user_id".to_string()],
        &[
            (
                "event_count".to_string(),
                "target.event_count + delta.event_count".to_string(),
            ),
            (
                "first_seen".to_string(),
                "LEAST(target.first_seen, delta.first_seen)".to_string(),
            ),
            (
                "last_seen".to_string(),
                "GREATEST(target.last_seen, delta.last_seen)".to_string(),
            ),
        ],
        "SELECT device_id, user_id, COUNT(*) AS event_count, MIN(event_ts) AS first_seen, \
         MAX(event_ts) AS last_seen FROM events GROUP BY 1, 2",
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(group.statements.len(), 1);
    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO main.device_user_edges AS target USING (SELECT device_id, user_id, \
         COUNT(*) AS event_count, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
         FROM events GROUP BY 1, 2) AS delta \
         ON target.device_id = delta.device_id AND target.user_id = delta.user_id \
         WHEN MATCHED THEN UPDATE SET event_count = target.event_count + delta.event_count, \
         first_seen = LEAST(target.first_seen, delta.first_seen), \
         last_seen = GREATEST(target.last_seen, delta.last_seen) \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

/// The first-run `CREATE TABLE … AS` for a windowed-keyed-maintenance cell
/// (`maintenance_driver::run_windowed_keyed_maintenance`'s create arm):
/// the target does not exist yet, so the first step's delta becomes the
/// table wholesale.
#[test]
fn create_table_as_matches_production_shape() {
    let group = emit_create_table_as(
        "main.device_user_edges",
        "SELECT device_id, user_id, COUNT(*) AS event_count FROM events GROUP BY 1, 2",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group.statements.len(), 1);
    assert!(
        !group.transactional,
        "a single-statement group needs no transaction wrapper"
    );
    assert_eq!(
        group.statements[0].sql,
        "CREATE TABLE main.device_user_edges AS SELECT device_id, user_id, COUNT(*) AS \
         event_count FROM events GROUP BY 1, 2"
    );
}
