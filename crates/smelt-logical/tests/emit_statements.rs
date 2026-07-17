//! Correctness oracle for `smelt_logical::maintenance::emit`
//! (`docs/specs/incremental_models.md` §"Statement emission (single owner)"):
//! the emitters are the *single author* of every maintenance statement a
//! run executes. This file asserts each emitter's output shape directly
//! (byte-parity against production text is asserted from the execution side
//! in `crates/smelt-runtime/tests/statement_parity.rs`).

use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_create_table_as, emit_delete_insert, emit_keyed_fold,
    MaintenanceDialect, Region,
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
        None,
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

/// A locality-admitted keyed fold (`docs/specs/incremental_models.md`
/// §"Key temporal locality") carries an extra target-side partition
/// predicate on the `ON` condition — restricting which target rows the
/// `MERGE` scans/matches without changing which delta rows merge (every
/// row in `delta_select` still merges, per "Pruning is not a write
/// clamp").
#[test]
fn keyed_fold_with_slice_carries_target_partition_predicate() {
    let slice = smelt_logical::maintenance::emit::TargetSlicePredicate {
        partition_column: "event_date".to_string(),
        lower: "2026-01-02".to_string(),
        upper: "2026-01-02".to_string(),
    };
    let group = emit_keyed_fold(
        "main.device_daily",
        &["device_id".to_string(), "event_date".to_string()],
        &[(
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        )],
        "SELECT device_id, event_date, COUNT(*) AS event_count FROM events GROUP BY 1, 2",
        Some(&slice),
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO main.device_daily AS target USING (SELECT device_id, event_date, \
         COUNT(*) AS event_count FROM events GROUP BY 1, 2) AS delta \
         ON target.device_id = delta.device_id AND target.event_date = delta.event_date \
         AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02' \
         WHEN MATCHED THEN UPDATE SET event_count = target.event_count + delta.event_count \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

/// A quote in a slice bound is escaped, matching every other emitter in
/// this module (`delete_insert_escapes_quoted_literal_region_boundaries`).
#[test]
fn keyed_fold_slice_escapes_quoted_literal_bounds() {
    let slice = smelt_logical::maintenance::emit::TargetSlicePredicate {
        partition_column: "name".to_string(),
        lower: "O'Brien".to_string(),
        upper: "Z".to_string(),
    };
    let group = emit_keyed_fold(
        "main.t",
        &["id".to_string()],
        &[],
        "SELECT id, name FROM events",
        Some(&slice),
        MaintenanceDialect::DuckDb,
    );
    assert!(
        group.statements[0]
            .sql
            .contains("AND target.name BETWEEN 'O''Brien' AND 'Z'"),
        "expected escaped slice bound: {}",
        group.statements[0].sql
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

/// The column-scoped `MERGE` production actually executes for `Technique::
/// ColumnScopedMerge` (`crate::maintenance_driver::execute_column_scoped_merge`/
/// `execute_column_scoped_merge_full`'s pre-phase text, both DuckDB's
/// `merge_into` and `smelt-backend-spark::sql::merge_into`'s pre-phase
/// text): `UPDATE SET *`, `INSERT *` — no explicit column list, matching
/// every backend's production shape byte-for-byte. There is no
/// partition-bounded variant: partition-scoping, when the technique is not
/// the declared full-scan case, is the caller's job, folded into
/// `source_select` before it reaches the emitter.
#[test]
fn column_scoped_merge_duckdb_uses_set_star_full_row_projection() {
    let group = emit_column_scoped_merge(
        "main.daily_events_enriched",
        &["event_id".to_string()],
        "SELECT event_id, event_date, user_id, event_type, user_name \
         FROM sources_raw_events e JOIN sources_raw_users u ON e.user_id = u.user_id",
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(group.statements.len(), 1);
    assert!(
        !group.transactional,
        "a single-statement group needs no transaction wrapper"
    );
    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO main.daily_events_enriched AS target USING (SELECT event_id, event_date, \
         user_id, event_type, user_name FROM sources_raw_events e JOIN sources_raw_users u ON \
         e.user_id = u.user_id) AS source ON target.event_id = source.event_id \
         WHEN MATCHED THEN UPDATE SET * \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

/// A composite `unique_key` renders every key column into the `ON` clause,
/// `AND`-joined, matching both DuckDB's and Spark's pre-phase production
/// text for a multi-column key.
#[test]
fn column_scoped_merge_composite_key_ands_every_column() {
    let group = emit_column_scoped_merge(
        "cat.db.events",
        &["user_id".to_string(), "event_date".to_string()],
        "SELECT * FROM staging",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO cat.db.events AS target USING (SELECT * FROM staging) AS source ON \
         target.user_id = source.user_id AND target.event_date = source.event_date \
         WHEN MATCHED THEN UPDATE SET * \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

/// The column-scoped MERGE family shares identical grammar across DuckDB
/// and Spark today — both backends' pre-phase `merge_into` text was
/// byte-identical (`smelt-backend-duckdb::merge_into`,
/// `smelt-backend-spark::sql::merge_into`) — so the dialect-keyed variants
/// coincide, same precedent as `emit_delete_insert`'s dialect-invariant
/// shape.
#[test]
fn column_scoped_merge_dialect_invariant_shape() {
    let duckdb = emit_column_scoped_merge(
        "t",
        &["id".to_string()],
        "SELECT 1 AS id",
        MaintenanceDialect::DuckDb,
    );
    let spark = emit_column_scoped_merge(
        "t",
        &["id".to_string()],
        "SELECT 1 AS id",
        MaintenanceDialect::Spark,
    );
    assert_eq!(duckdb, spark);
}
