//! Correctness oracle for `smelt_logical::maintenance::emit`
//! (`docs/specs/incremental_models.md` §"Statement emission (single owner)"):
//! the emitters are the *single author* of every maintenance statement a
//! run executes. This file asserts each emitter's output shape directly
//! (byte-parity against production text is asserted from the execution side
//! in `crates/smelt-runtime/tests/statement_parity.rs`).

use smelt_logical::analysis::decomposed_state::StateColumn;
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_create_table_as, emit_delete_insert, emit_in_place_update,
    emit_keyed_fold, emit_recurrence_bound_probe, emit_staged_candidate_conditional,
    emit_staged_candidate_conditional_recompute, state_augmented_projection, MaintenanceDialect,
    Region, StateAugmentRefusal,
};
use smelt_logical::CrossPartitionCombiner;

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
    let slice = smelt_logical::maintenance::emit::TargetSlicePredicate::Range {
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
    let slice = smelt_logical::maintenance::emit::TargetSlicePredicate::Range {
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

/// Route 2 (key-determined) locality slices the target scan by the delta's
/// own partition values (`docs/specs/incremental_models.md` §"Key temporal
/// locality", route 2), rendered as an `IN (SELECT DISTINCT … FROM
/// (<delta>))` predicate rather than route 1's literal `BETWEEN` range —
/// no widening, no caller-precomputed bounds.
#[test]
fn keyed_fold_with_delta_values_slice_carries_in_subquery_predicate() {
    let delta_select = "SELECT event_id, MIN(event_date) AS first_seen_date FROM events GROUP BY 1";
    let slice = smelt_logical::maintenance::emit::TargetSlicePredicate::DeltaValues {
        partition_column: "first_seen_date".to_string(),
        delta_select: delta_select.to_string(),
    };
    let group = emit_keyed_fold(
        "main.events_deduped",
        &["event_id".to_string()],
        &[],
        delta_select,
        Some(&slice),
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO main.events_deduped AS target USING (SELECT event_id, MIN(event_date) AS \
         first_seen_date FROM events GROUP BY 1) AS delta ON target.event_id = delta.event_id \
         AND target.first_seen_date IN (SELECT DISTINCT first_seen_date FROM (SELECT event_id, \
         MIN(event_date) AS first_seen_date FROM events GROUP BY 1) AS __locality_delta_values) \
         WHEN MATCHED THEN UPDATE SET  WHEN NOT MATCHED THEN INSERT *"
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

/// The Spark twin of [`create_table_as_matches_production_shape`]: the
/// first-run bootstrap CREATE for a keyed-fold/column-scoped-merge cell must
/// itself create a Delta-formatted table (`USING DELTA`), since every
/// following step for that same cell is a `MERGE INTO` — a plain
/// (default-format, non-Delta) Spark-managed table cannot be the target of a
/// `MERGE` (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 4:
/// discovered via a live-Spark equivalence-leg failure —
/// `UnsupportedOperationException: MERGE INTO TABLE is not supported
/// temporarily` — when the bootstrap CREATE omitted the format clause).
#[test]
fn create_table_as_spark_dialect_specifies_delta_format() {
    let group = emit_create_table_as(
        "smelt_conf_gen.device_user_edges",
        "SELECT device_id, user_id, COUNT(*) AS event_count FROM events GROUP BY 1, 2",
        MaintenanceDialect::Spark,
    );
    assert_eq!(group.statements.len(), 1);
    assert_eq!(
        group.statements[0].sql,
        "CREATE TABLE smelt_conf_gen.device_user_edges USING DELTA AS SELECT device_id, \
         user_id, COUNT(*) AS event_count FROM events GROUP BY 1, 2"
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

/// The route-3 (recurrence-bounded, declared `r`) out-of-slice match probe
/// (`docs/specs/incremental_models.md` §"Key temporal locality", route 3):
/// a single read-only `SELECT` counting keys the delta shares with a
/// stored target row whose partition column lies before the slice's lower
/// bound, plus up to 5 sample violating keys.
#[test]
fn recurrence_bound_probe_matches_production_shape() {
    let delta_select = "SELECT event_id, event_date FROM events WHERE event_date = '2026-01-10'";
    let stmt = emit_recurrence_bound_probe(
        "main.events_last_seen",
        &["event_id".to_string()],
        "last_seen_date",
        delta_select,
        "2026-01-07",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        stmt.sql,
        "WITH __recurrence_violations AS (SELECT DISTINCT CAST(target.event_id AS VARCHAR) AS \
         violation_key FROM main.events_last_seen AS target JOIN (SELECT DISTINCT event_id FROM \
         (SELECT event_id, event_date FROM events WHERE event_date = '2026-01-10')) AS delta ON \
         target.event_id = delta.event_id WHERE target.last_seen_date < '2026-01-07') SELECT \
         COUNT(*) AS violation_count, (SELECT STRING_AGG(violation_key, ', ') FROM (SELECT \
         violation_key FROM __recurrence_violations LIMIT 5) AS __sample) AS sample_keys FROM \
         __recurrence_violations"
    );
}

/// A composite key concatenates every key column into one violation-key
/// string (`k1 || '|' || k2 || ...`), and the join condition ANDs every
/// key column — mirroring `emit_keyed_fold`'s own composite-key handling.
#[test]
fn recurrence_bound_probe_composite_key_concatenates_and_ands() {
    let delta_select = "SELECT tenant_id, event_id, event_date FROM events";
    let stmt = emit_recurrence_bound_probe(
        "main.t",
        &["tenant_id".to_string(), "event_id".to_string()],
        "last_seen_date",
        delta_select,
        "2026-01-01",
        MaintenanceDialect::DuckDb,
    );
    assert!(
        stmt.sql.contains(
            "CAST(target.tenant_id AS VARCHAR) || '|' || CAST(target.event_id AS VARCHAR)"
        ),
        "expected composite key concatenation in: {}",
        stmt.sql
    );
    assert!(
        stmt.sql
            .contains("target.tenant_id = delta.tenant_id AND target.event_id = delta.event_id"),
        "expected ANDed composite join condition in: {}",
        stmt.sql
    );
}

/// A single-quote in the slice-lower literal is escaped, matching every
/// other emitter's literal-escaping convention in this module.
#[test]
fn recurrence_bound_probe_escapes_quoted_slice_lower() {
    let stmt = emit_recurrence_bound_probe(
        "main.t",
        &["event_id".to_string()],
        "last_seen_date",
        "SELECT event_id, event_date FROM events",
        "2026-01-0'7",
        MaintenanceDialect::DuckDb,
    );
    assert!(
        stmt.sql.contains("< '2026-01-0''7'"),
        "expected escaped literal in: {}",
        stmt.sql
    );
}

/// `MaintenanceDialect::Spark` uses `STRING` (not DuckDB's unsized
/// `VARCHAR`, which Spark's parser refuses with `DATATYPE_MISSING_SIZE`) and
/// `CONCAT_WS(', ', COLLECT_LIST(...))` (Spark has no `STRING_AGG`) — a real
/// bug found while porting the recurrence-bounded composed-pool conformance
/// leg to live Spark
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5): the
/// probe's DuckDB-only SQL shape made every route-3 checked-merge step fail
/// with a Spark `ParseException` before the merge itself ever ran.
#[test]
fn recurrence_bound_probe_spark_dialect_uses_string_and_concat_ws() {
    let stmt = emit_recurrence_bound_probe(
        "smelt_conf_gen.t",
        &["id".to_string()],
        "last_seen",
        "SELECT id, d FROM events",
        "2026-01-01",
        MaintenanceDialect::Spark,
    );
    assert!(
        stmt.sql.contains("CAST(target.id AS STRING)"),
        "expected Spark's unsized STRING cast, got: {}",
        stmt.sql
    );
    assert!(
        !stmt.sql.contains("VARCHAR"),
        "Spark dialect must never emit VARCHAR (DATATYPE_MISSING_SIZE): {}",
        stmt.sql
    );
    assert!(
        stmt.sql
            .contains("CONCAT_WS(', ', COLLECT_LIST(violation_key))"),
        "expected Spark's STRING_AGG-equivalent aggregate, got: {}",
        stmt.sql
    );
    assert!(
        !stmt.sql.contains("STRING_AGG"),
        "Spark dialect must never emit STRING_AGG (not a Spark SQL builtin): {}",
        stmt.sql
    );
}

/// Substrate unification (`docs/plans/20260808-substrate-unification.md`
/// Phase 6): backbuild's unregioned in-place `UPDATE` is not a second,
/// forked emitter — it is this module's [`emit_in_place_update`] called
/// with an absent region. Byte-identical by construction, asserted here
/// rather than assumed.
#[test]
fn backbuild_unregioned_update_is_the_maintenance_emitter() {
    let assignments = vec![(
        "referrer_domain".to_string(),
        "regexp_extract(referrer, '://([^/]+)', 1)".to_string(),
    )];

    let backbuild_sql =
        smelt_logical::backbuild::emit::emit_in_place_update("clickstream", &assignments);
    let maintenance_sql = emit_in_place_update("clickstream", &assignments, None);

    assert_eq!(
        maintenance_sql.len(),
        1,
        "an unregioned in-place update is still exactly one statement"
    );
    assert_eq!(backbuild_sql, maintenance_sql[0]);
}

/// The regioned form still carries the `WHERE` clause — the `Option<Region>`
/// generalization must not silently drop it.
#[test]
fn maintenance_regioned_update_still_carries_the_where_clause() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-02-01'".to_string(),
    };
    let assignments = vec![("total".to_string(), "price * qty".to_string())];
    let stmts = emit_in_place_update("t", &assignments, Some(("event_date", &region)));
    assert_eq!(stmts.len(), 1);
    assert!(stmts[0].starts_with("UPDATE t SET total = price * qty WHERE"));
    assert!(stmts[0].contains("event_date >= DATE '2026-01-01'"));
}

/// `emit_staged_candidate_conditional_recompute`
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 3): six
/// statements — the same five as [`emit_staged_candidate_conditional`], plus
/// one extra `DELETE` (inserted between the matched-and-changed `DELETE` and
/// the reinsert `INSERT`) that removes every stored row whose key is absent
/// from the staged candidate entirely.
#[test]
fn staged_candidate_conditional_recompute_adds_a_departed_key_delete() {
    let key = vec!["user_id".to_string()];
    let compared_columns = vec!["tier".to_string()];
    let candidate_select = "SELECT * FROM (VALUES (1, 'bronze')) AS t(user_id, tier)";

    let region_scoped = emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    let recompute = emit_staged_candidate_conditional_recompute(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(region_scoped.statements.len(), 5);
    assert_eq!(recompute.statements.len(), 6);
    assert!(recompute.transactional);

    // CREATE, INSERT candidates, and the matched-and-changed DELETE are
    // byte-identical between the two variants.
    assert_eq!(recompute.statements[0], region_scoped.statements[0]);
    assert_eq!(recompute.statements[1], region_scoped.statements[1]);
    assert_eq!(recompute.statements[2], region_scoped.statements[2]);

    // The extra departed-key DELETE sits between the matched-changed DELETE
    // and the reinsert INSERT.
    assert_eq!(
        recompute.statements[3].sql,
        "DELETE FROM main.dim_users WHERE NOT EXISTS (SELECT 1 FROM \
         __smelt_staged_dim_users AS s WHERE main.dim_users.user_id = s.user_id)"
    );

    // The reinsert INSERT and the final DROP are byte-identical to the
    // region-scoped variant's own trailing two statements.
    assert_eq!(recompute.statements[4], region_scoped.statements[3]);
    assert_eq!(recompute.statements[5], region_scoped.statements[4]);
}

#[test]
#[should_panic(expected = "non-empty row identity")]
fn staged_candidate_conditional_recompute_panics_on_empty_key() {
    emit_staged_candidate_conditional_recompute(
        "main.t",
        "__staged",
        &[],
        "SELECT 1",
        &["a".to_string()],
        MaintenanceDialect::DuckDb,
    );
}

/// A state-bearing fold set (`docs/specs/incremental_models.md` §"Decomposed
/// state (rung 2) in keyed models"): `emit_keyed_fold` assembles the `SET`
/// clause from whatever `(column, expr)` pairs it is handed — the state
/// columns' own combiner expressions, plus the presented column set to `π`
/// over the merged state exprs (`smelt-runtime`'s
/// `build_cumulative_merge_sql` is the caller that derives this expanded
/// fold list; this test pins the emitter's generic handling of that shape).
#[test]
fn keyed_fold_over_state_projects_and_folds_state_columns() {
    let group = emit_keyed_fold(
        "main.customer_stats",
        &["customer_id".to_string()],
        &[
            (
                "avg_amount__sum".to_string(),
                "target.avg_amount__sum + delta.avg_amount__sum".to_string(),
            ),
            (
                "avg_amount__count".to_string(),
                "target.avg_amount__count + delta.avg_amount__count".to_string(),
            ),
            (
                "avg_amount".to_string(),
                "(target.avg_amount__sum + delta.avg_amount__sum) / \
                 (target.avg_amount__count + delta.avg_amount__count)"
                    .to_string(),
            ),
        ],
        "SELECT customer_id, SUM(amount) AS avg_amount__sum, COUNT(amount) AS avg_amount__count \
         FROM events GROUP BY 1",
        None,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements[0].sql,
        "MERGE INTO main.customer_stats AS target USING (SELECT customer_id, SUM(amount) AS \
         avg_amount__sum, COUNT(amount) AS avg_amount__count FROM events GROUP BY 1) AS delta \
         ON target.customer_id = delta.customer_id \
         WHEN MATCHED THEN UPDATE SET \
         avg_amount__sum = target.avg_amount__sum + delta.avg_amount__sum, \
         avg_amount__count = target.avg_amount__count + delta.avg_amount__count, \
         avg_amount = (target.avg_amount__sum + delta.avg_amount__sum) / \
         (target.avg_amount__count + delta.avg_amount__count) \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

/// `state_augmented_projection` appends `, <per_partition_expr> AS <state
/// col>` for each state column, leaving the key/GROUP BY and the model's own
/// presented select item unchanged.
#[test]
fn state_augmented_projection_appends_state_select_items() {
    let sql = "SELECT customer_id, SUM(amount) / COUNT(amount) AS avg_amount FROM events \
               GROUP BY customer_id";
    let state_columns = vec![
        StateColumn {
            name: "avg_amount__sum".to_string(),
            per_partition_expr: "SUM(amount)".to_string(),
            combiner: CrossPartitionCombiner::Sum,
        },
        StateColumn {
            name: "avg_amount__count".to_string(),
            per_partition_expr: "COUNT(amount)".to_string(),
            combiner: CrossPartitionCombiner::Sum,
        },
    ];
    let augmented =
        state_augmented_projection(sql, &state_columns).expect("well-formed SQL must augment");
    assert_eq!(
        augmented,
        "SELECT customer_id, SUM(amount) / COUNT(amount) AS avg_amount, SUM(amount) AS \
         avg_amount__sum, COUNT(amount) AS avg_amount__count FROM events GROUP BY customer_id"
    );

    // An empty state-column list returns the SQL unchanged (the stateless
    // shape every column family admitted before this mechanism existed
    // still produces).
    assert_eq!(
        state_augmented_projection(sql, &[]).expect("empty state must round-trip unchanged"),
        sql
    );
}

/// Unparseable SQL is refused, never mangled into a broken string.
#[test]
fn state_augmented_projection_refuses_unparseable_sql() {
    let state_columns = vec![StateColumn {
        name: "x__sum".to_string(),
        per_partition_expr: "SUM(x)".to_string(),
        combiner: CrossPartitionCombiner::Sum,
    }];
    let result = state_augmented_projection("not even sql (((", &state_columns);
    assert_eq!(result, Err(StateAugmentRefusal::Unparseable));
}
