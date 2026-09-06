//! Driver-level tests for the succession-patch technique
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/05b-plan.md`).
//! Tests 1-8 exercise [`execute_succession_maintenance`] directly against a
//! real DuckDB, with a hand-built [`SuccessionCell`] (mirroring
//! `crates/smelt-logical/tests/succession_emit.rs`'s own precedent of
//! driving emitters from hand-built recipes) — never through
//! `resolve_live_succession_cell`, which test 9 alone exercises.

use std::path::{Path, PathBuf};

use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, RefreshStrategy, TimeseriesConfig};
use smelt_core::sources::{
    MutationProfile as SourceMutationKind, SourceColumn, SourceInfo, SourceMutationProfile,
};
use smelt_core::ModelMetadata;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::succession::SuccessionRecipe;
use smelt_types::DataType;

use super::{
    execute_succession_maintenance, rebuild_succession_state, resolve_live_succession_cell,
    SuccessionCell,
};
use crate::maintenance_driver::driving_steps;

const NO_OP_REPORTER: crate::reporter::NoOpReporter = crate::reporter::NoOpReporter;

fn no_retry_policy() -> crate::execute::RetryPolicy<'static> {
    crate::execute::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "succession-unit-test",
        model_name: "customer_history",
        reporter: &NO_OP_REPORTER,
    }
}

fn probe_policy() -> crate::probes::ProbePolicy {
    crate::probes::ProbePolicy::per_run()
}

fn ts(with_tz: bool) -> DataType {
    DataType::Timestamp {
        with_timezone: with_tz,
    }
}

fn varchar() -> DataType {
    DataType::Varchar { max_length: None }
}

/// The presented table's resolved output schema
/// (`UpstreamSchemas.models["customer_history"]`), used both to bootstrap
/// the empty shell and to type the tombstone ledger's `key_cols ++
/// [clock_col]`.
fn presented_columns() -> Vec<(String, DataType)> {
    vec![
        ("customer_id".to_string(), DataType::Integer),
        ("changed_at".to_string(), ts(false)),
        ("tier".to_string(), varchar()),
        ("valid_to".to_string(), ts(false)),
    ]
}

/// A hand-built recipe mirroring what `SuccessionRecipe::from_verdict` would
/// assemble for `customer_id, changed_at, tier[, is_deleted], LEAD(changed_at)
/// AS valid_to FROM raw.customer_changes` — built directly (not derived from
/// a classified model) so these tests control every field precisely, same
/// precedent as `succession_emit.rs`'s own hand-built recipes.
fn recipe(with_delete: bool) -> SuccessionRecipe {
    let mut row_local = vec![
        ("customer_id".to_string(), "customer_id".to_string()),
        ("changed_at".to_string(), "changed_at".to_string()),
        ("tier".to_string(), "tier".to_string()),
    ];
    if with_delete {
        row_local.push(("is_deleted".to_string(), "is_deleted".to_string()));
    }
    SuccessionRecipe {
        source_table: "customer_changes".to_string(),
        pre_filter: None,
        key_cols: vec!["customer_id".to_string()],
        clock_col: "changed_at".to_string(),
        payload_columns: vec!["tier".to_string()],
        row_local_projection: row_local,
        lead_derived: vec![("valid_to".to_string(), "{lead}".to_string())],
        lag_derived: vec![],
        delete_flag_expr: if with_delete {
            Some("is_deleted".to_string())
        } else {
            None
        },
    }
}

/// Pre-create the presented and tombstone tables by hand, with `valid_to`
/// declared `NOT NULL` — a key with no successor event produces a `NULL`
/// `valid_to`, so the presented `MERGE`'s `INSERT` violates the constraint
/// and the whole statement fails, exercising the transactional-rollback leg
/// (test 7). The probe's own `SELECT` never touches `valid_to` (it is not a
/// payload column), so this failure is specific to the write, not the
/// read-only probe.
fn stage_presented_with_not_null_valid_to(conn: &duckdb::Connection) {
    conn.execute_batch(
        "CREATE TABLE main.customer_history (customer_id INTEGER, changed_at TIMESTAMP, tier \
         VARCHAR, valid_to TIMESTAMP NOT NULL)",
    )
    .expect("create presented table with NOT NULL valid_to");
}

fn cell(recipe: SuccessionRecipe) -> SuccessionCell {
    SuccessionCell {
        recipe,
        presented_table: "main.customer_history".to_string(),
        source_table: "main.raw_customer_changes".to_string(),
        partition_column: "arrival_date".to_string(),
        granularity: Granularity::Day,
    }
}

async fn open_backend(db_path: &Path) -> DuckDbBackend {
    DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb")
}

fn stage_source(conn: &duckdb::Connection) {
    conn.execute_batch(
        "CREATE TABLE main.raw_customer_changes (customer_id INTEGER, changed_at TIMESTAMP, \
         arrival_date DATE, tier VARCHAR, is_deleted BOOLEAN)",
    )
    .expect("create source table");
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    conn: &duckdb::Connection,
    id: i64,
    changed_at: &str,
    arrival_date: &str,
    tier: &str,
    deleted: bool,
) {
    conn.execute_batch(&format!(
        "INSERT INTO main.raw_customer_changes VALUES ({id}, TIMESTAMP '{changed_at}', DATE \
         '{arrival_date}', '{tier}', {deleted})"
    ))
    .expect("insert event");
}

fn row_count(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM ({sql}) AS t"), [], |row| {
        row.get(0)
    })
    .expect("row count query")
}

async fn run_steps(
    db_path: &Path,
    steps: &[crate::maintenance_driver::MaintenanceStep],
    cell: &SuccessionCell,
) -> anyhow::Result<()> {
    let backend = open_backend(db_path).await;
    execute_succession_maintenance(
        &backend,
        "customer_history",
        "main",
        "customer_history",
        steps,
        cell,
        &presented_columns(),
        &no_retry_policy(),
        &probe_policy(),
        &NO_OP_REPORTER,
        "run-1",
    )
    .await?;
    Ok(())
}

fn day(s: &str, e: &str) -> Vec<crate::maintenance_driver::MaintenanceStep> {
    driving_steps(s, e, &Granularity::Day).expect("driving_steps")
}

/// The model's own compiled full-refresh oracle for [`recipe(true)`] —
/// `QUALIFY NOT is_deleted` excludes delete-flagged rows from the presented
/// arm, matching `crates/smelt-logical/tests/succession_emit.rs`'s
/// `oracle_sql(true)`.
const REBUILD_MODEL_SQL: &str = "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER \
     (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM main.raw_customer_changes \
     QUALIFY NOT is_deleted";

async fn run_rebuild(
    db_path: &Path,
    model_select_sql: &str,
    cell: &SuccessionCell,
) -> anyhow::Result<()> {
    let backend = open_backend(db_path).await;
    rebuild_succession_state(
        &backend,
        "customer_history",
        "main",
        "customer_history",
        cell,
        &presented_columns(),
        model_select_sql,
        &no_retry_policy(),
        &NO_OP_REPORTER,
        "run-rebuild",
    )
    .await?;
    Ok(())
}

fn table_exists(conn: &duckdb::Connection, schema: &str, table: &str) -> bool {
    row_count(
        conn,
        &format!(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = '{schema}' AND \
             table_name = '{table}'"
        ),
    ) > 0
}

/// Test 1: `first_window_bootstraps_shell_then_patches` — an empty presented
/// table and an empty tombstone table exist after the run, and the first
/// window's row lands through the `MERGE` (the row is present with the
/// correct value; the implementation never emits `CREATE TABLE ... AS` for
/// the succession grain at all — see `execute.rs`'s doc comment).
#[tokio::test]
async fn first_window_bootstraps_shell_then_patches() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
    }
    let steps = day("2026-01-01", "2026-01-02");
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("run succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        1,
        "the first window's row must land through the MERGE"
    );
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history__tombstones"),
        0
    );
    let tier: String = conn
        .query_row(
            "SELECT tier FROM main.customer_history WHERE customer_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read tier");
    assert_eq!(tier, "gold");
    let valid_to: Option<String> = conn
        .query_row(
            "SELECT CAST(valid_to AS VARCHAR) FROM main.customer_history WHERE customer_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read valid_to");
    assert!(
        valid_to.is_none(),
        "the only event for this key has no successor: {valid_to:?}"
    );
}

/// Test 2: `refolding_one_window_is_byte_identical` — applying window W
/// twice leaves the presented row and the ledger row unchanged, and the
/// second run succeeds (no `KeyedReprocessedWindow`-shaped refusal — the
/// succession grain's merge ledger is re-run-tolerant, `ON CONFLICT DO
/// NOTHING`, never a never-fold-twice `PRIMARY KEY` refusal).
#[tokio::test]
async fn refolding_one_window_is_byte_identical() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
    }
    let steps = day("2026-01-01", "2026-01-02");
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("first run succeeds");
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("refold of the same window must succeed, not refuse");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        1,
        "refolding must not duplicate the row"
    );
    let tier: String = conn
        .query_row(
            "SELECT tier FROM main.customer_history WHERE customer_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read tier");
    assert_eq!(tier, "gold");
}

/// Test 3: `two_windows_converge_in_either_order` — W1 then W2 equals W2
/// then W1, both equal to the model SQL's own full-refresh oracle over
/// W1 ∪ W2.
#[tokio::test]
async fn two_windows_converge_in_either_order() {
    async fn run_order(order: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("run.duckdb");
        {
            let conn = duckdb::Connection::open(&db_path).expect("open");
            stage_source(&conn);
            insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
            insert_event(
                &conn,
                1,
                "2026-01-02 08:00:00",
                "2026-01-02",
                "silver",
                false,
            );
        }
        for (s, e) in order {
            let steps = day(s, e);
            run_steps(&db_path, &steps, &cell(recipe(true)))
                .await
                .expect("run succeeds");
        }
        (tmp, db_path)
    }

    let (_tmp_fwd, fwd_path) =
        run_order(&[("2026-01-01", "2026-01-02"), ("2026-01-02", "2026-01-03")]).await;
    let (_tmp_rev, rev_path) =
        run_order(&[("2026-01-02", "2026-01-03"), ("2026-01-01", "2026-01-02")]).await;

    let fwd_conn = duckdb::Connection::open(&fwd_path).expect("open fwd");
    let rev_conn = duckdb::Connection::open(&rev_path).expect("open rev");
    let oracle = "SELECT customer_id, changed_at, tier, \
                  LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS \
                  valid_to FROM main.raw_customer_changes";

    let fwd_count = row_count(&fwd_conn, "SELECT * FROM main.customer_history");
    let rev_count = row_count(&rev_conn, "SELECT * FROM main.customer_history");
    assert_eq!(fwd_count, 2);
    assert_eq!(rev_count, 2);

    let oracle_count = row_count(&fwd_conn, oracle);
    assert_eq!(
        oracle_count, 2,
        "sanity: the oracle over the shared source must also see both rows"
    );

    let fwd_diff = fwd_conn
        .query_row(
            &format!(
                "SELECT count(*) FROM ((SELECT customer_id, changed_at, tier, valid_to FROM \
                 main.customer_history) EXCEPT ALL ({oracle})) AS d"
            ),
            [],
            |r| r.get::<_, i64>(0),
        )
        .expect("fwd diff");
    assert_eq!(
        fwd_diff, 0,
        "forward order must match the full-refresh oracle"
    );

    let rev_diff = rev_conn
        .query_row(
            &format!(
                "SELECT count(*) FROM ((SELECT customer_id, changed_at, tier, valid_to FROM \
                 main.customer_history) EXCEPT ALL ({oracle})) AS d"
            ),
            [],
            |r| r.get::<_, i64>(0),
        )
        .expect("rev diff");
    assert_eq!(
        rev_diff, 0,
        "reverse order must match the full-refresh oracle"
    );
}

/// Test 4: `delete_event_lands_in_tombstone_ledger_not_presented` — a
/// delete-flagged row writes `(k, t)` to the tombstone ledger, is absent
/// from the presented table, and still splices its neighbour (the earlier
/// row's `valid_to` becomes the delete event's own `t`).
#[tokio::test]
async fn delete_event_lands_in_tombstone_ledger_not_presented() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        insert_event(&conn, 1, "2026-01-02 08:00:00", "2026-01-02", "gold", true);
    }
    let steps = day("2026-01-01", "2026-01-03");
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("run succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        1,
        "the delete event must not land in the presented table"
    );
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history__tombstones"),
        1,
        "the delete event must land in the tombstone ledger"
    );
    let valid_to: String = conn
        .query_row(
            "SELECT CAST(valid_to AS VARCHAR) FROM main.customer_history WHERE customer_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read valid_to");
    assert_eq!(
        valid_to, "2026-01-02 08:00:00",
        "the delete event must splice the earlier row's valid_to"
    );
}

/// Test 5: `clock_tie_refuses_before_any_write` — a non-identical second
/// row at the same `(k, t)` bails with `SuccessionClockTie`, naming the key
/// and clock columns; the presented and tombstone tables are unchanged from
/// their pre-run state.
#[tokio::test]
async fn clock_tie_refuses_before_any_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 2, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
    }
    // Establish the tables via a clean first window (a different key) so
    // the probe's `FROM {presented_table}` has something to read.
    run_steps(
        &db_path,
        &day("2026-01-01", "2026-01-02"),
        &cell(recipe(true)),
    )
    .await
    .expect("setup run succeeds");
    let pre_presented;
    let pre_tombstones;
    {
        let conn = duckdb::Connection::open(&db_path).expect("reopen");
        pre_presented = row_count(&conn, "SELECT * FROM main.customer_history");
        pre_tombstones = row_count(&conn, "SELECT * FROM main.customer_history__tombstones");
        conn.execute_batch(
            "INSERT INTO main.raw_customer_changes VALUES (1, TIMESTAMP '2026-01-02 08:00:00', \
             DATE '2026-01-02', 'gold', false), (1, TIMESTAMP '2026-01-02 08:00:00', DATE \
             '2026-01-02', 'silver', false)",
        )
        .expect("insert colliding events");
    }

    let err = run_steps(
        &db_path,
        &day("2026-01-02", "2026-01-03"),
        &cell(recipe(true)),
    )
    .await
    .expect_err("a clock tie must refuse before any write");
    let message = err.to_string();
    assert!(message.contains("SuccessionClockTie"), "{message}");
    assert!(message.contains("customer_id"), "{message}");
    assert!(message.contains("changed_at"), "{message}");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        pre_presented,
        "the presented table must be unchanged after a refused run"
    );
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history__tombstones"),
        pre_tombstones,
        "the tombstone table must be unchanged after a refused run"
    );
}

/// Test 6: `identical_represented_row_is_a_no_op` — the same `(k, t)`
/// redelivered (byte-identical content and delete flag) across two
/// different arrival windows is not a clock tie and does not change the
/// stored row.
#[tokio::test]
async fn identical_represented_row_is_a_no_op() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        // Same (customer_id, changed_at, tier, is_deleted) landing in two
        // different arrival-date windows — a redelivered-identical event.
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-02", "gold", false);
    }
    let steps = day("2026-01-01", "2026-01-03");
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("redelivered-identical rows must not refuse");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        1,
        "a redelivered-identical row must not duplicate the presented row"
    );
    let tier: String = conn
        .query_row(
            "SELECT tier FROM main.customer_history WHERE customer_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read tier");
    assert_eq!(tier, "gold");
}

/// Test 7: `failed_merge_rolls_back_the_tombstone_insert` — a recipe whose
/// `MERGE` fails (an `INSERT` violating the presented table's own `NOT
/// NULL valid_to` constraint, since customer 1's single event has no
/// successor) leaves the tombstone table's row count unchanged: customer
/// 2's delete event's tombstone insert shares one transaction with the
/// failing `MERGE`, so it rolls back too.
#[tokio::test]
async fn failed_merge_rolls_back_the_tombstone_insert() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        stage_presented_with_not_null_valid_to(&conn);
        // Customer 1: a single event with no successor -> NULL valid_to on
        // INSERT -> violates the NOT NULL constraint.
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        // Customer 2: a delete event -> tombstone-worthy, in the SAME
        // transactional patch group as customer 1's failing MERGE.
        insert_event(&conn, 2, "2026-01-01 09:00:00", "2026-01-01", "gold", true);
    }
    let steps = day("2026-01-01", "2026-01-02");
    let err = run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect_err("an INSERT violating the NOT NULL valid_to constraint must fail");
    assert!(
        err.to_string()
            .contains("Failed to execute succession-patch"),
        "{err}"
    );

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history__tombstones"),
        0,
        "the tombstone insert must roll back with the failed MERGE"
    );
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        0,
        "the presented table must be unchanged after the failed MERGE"
    );
}

/// Test 8: `frontier_record_is_written_per_window` — the merge-ledger table
/// carries one row per applied window, keyed on the model and the step's
/// own partition value.
#[tokio::test]
async fn frontier_record_is_written_per_window() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        insert_event(
            &conn,
            1,
            "2026-01-02 08:00:00",
            "2026-01-02",
            "silver",
            false,
        );
    }
    let steps = day("2026-01-01", "2026-01-03");
    assert_eq!(steps.len(), 2);
    run_steps(&db_path, &steps, &cell(recipe(true)))
        .await
        .expect("run succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    let ledger_rows = row_count(
        &conn,
        "SELECT * FROM main._smelt_ledger WHERE model_name = 'customer_history'",
    );
    assert_eq!(ledger_rows, 2, "one ledger row per applied window");
}

/// Test (phase 5c, `docs/outcomes/20260906-scd2-keyed-succession/phases/
/// 05c-plan.md`) 3: `full_refresh_rebuilds_presented_and_ledger_from_source`
/// — after two patched windows, a full-rebuild call
/// ([`rebuild_succession_state`]) leaves both the presented table and the
/// tombstone ledger equal to their whole-source definitions.
#[tokio::test]
async fn full_refresh_rebuilds_presented_and_ledger_from_source() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        insert_event(
            &conn,
            1,
            "2026-01-02 08:00:00",
            "2026-01-02",
            "silver",
            true,
        );
    }
    run_steps(
        &db_path,
        &day("2026-01-01", "2026-01-03"),
        &cell(recipe(true)),
    )
    .await
    .expect("bootstrap run succeeds");

    run_rebuild(&db_path, REBUILD_MODEL_SQL, &cell(recipe(true)))
        .await
        .expect("full rebuild succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(
            &conn,
            "(SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history) EXCEPT \
             ALL (SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER (PARTITION BY \
             customer_id ORDER BY changed_at) AS valid_to FROM main.raw_customer_changes \
             QUALIFY NOT is_deleted)"
        ),
        0,
        "the rebuilt presented table must match the model SQL's own full-refresh oracle"
    );
    assert_eq!(
        row_count(
            &conn,
            "(SELECT customer_id, changed_at FROM main.customer_history__tombstones) EXCEPT ALL \
             (SELECT customer_id, changed_at FROM main.raw_customer_changes WHERE is_deleted)"
        ),
        0,
        "the rebuilt ledger must match the delete-flagged rows of the whole source"
    );
    assert_eq!(
        row_count(
            &conn,
            "(SELECT customer_id, changed_at FROM main.raw_customer_changes WHERE is_deleted) \
             EXCEPT ALL (SELECT customer_id, changed_at FROM \
             main.customer_history__tombstones)"
        ),
        0
    );
}

/// Test 4: `full_refresh_drops_a_tombstone_whose_source_row_vanished` — a
/// stale ledger row with no matching delete-flagged source row (seeded by
/// hand, simulating ledger drift a patch loop would never itself produce)
/// does not survive a full rebuild: the ledger arm is `DELETE` then
/// `INSERT ... SELECT`, never an append onto whatever the ledger already
/// held.
#[tokio::test]
async fn full_refresh_drops_a_tombstone_whose_source_row_vanished() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
    }
    run_steps(
        &db_path,
        &day("2026-01-01", "2026-01-02"),
        &cell(recipe(true)),
    )
    .await
    .expect("bootstrap run succeeds");
    {
        let conn = duckdb::Connection::open(&db_path).expect("reopen");
        conn.execute_batch(
            "INSERT INTO main.customer_history__tombstones (customer_id, changed_at) VALUES \
             (999, TIMESTAMP '2020-01-01 00:00:00')",
        )
        .expect("seed a stale tombstone with no matching source row");
    }

    run_rebuild(&db_path, REBUILD_MODEL_SQL, &cell(recipe(true)))
        .await
        .expect("full rebuild succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(
            &conn,
            "SELECT * FROM main.customer_history__tombstones WHERE customer_id = 999"
        ),
        0,
        "a stale tombstone with no matching delete-flagged source row must not survive a full \
         rebuild"
    );
}

/// Test 5: `failed_ledger_insert_rolls_back_the_presented_rebuild` — a
/// tombstone table pre-created with a `CHECK` constraint the ledger
/// rebuild's own `INSERT` violates fails the whole group; the presented
/// table's `CREATE TABLE ... AS`, sharing the same transaction, rolls back
/// too and never exists.
#[tokio::test]
async fn failed_ledger_insert_rolls_back_the_presented_rebuild() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", true);
        // `generate_tombstone_table_ddl` is `CREATE TABLE IF NOT EXISTS`, so
        // pre-creating the table here with a constraint the rebuild's own
        // INSERT violates survives the driver's own idempotent ensure-DDL
        // step untouched.
        conn.execute_batch(
            "CREATE TABLE main.customer_history__tombstones (customer_id INTEGER CHECK \
             (customer_id < 0), changed_at TIMESTAMP)",
        )
        .expect("seed a constrained tombstone table");
    }

    let err = run_rebuild(&db_path, REBUILD_MODEL_SQL, &cell(recipe(true)))
        .await
        .expect_err("a ledger INSERT violating its own CHECK constraint must fail");
    assert!(
        err.to_string()
            .contains("Failed to execute succession full-rebuild"),
        "{err}"
    );

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert!(
        !table_exists(&conn, "main", "customer_history"),
        "the presented table's CREATE TABLE AS shares a transaction with the failing ledger \
         INSERT and must roll back with it"
    );
}

/// Test 6: `range_rebuild_re_derives_the_whole_ledger` — a `smelt
/// rebuild`-shaped call (the compiled model SQL passed unscoped, as
/// `project.rs` would for a range rebuild once wired — see that file's own
/// doc comment on the current gap) ends with the ledger equal to the
/// whole-source rebuild `SELECT`, proving [`rebuild_succession_state`]
/// itself has no notion of a "range" — it is always a whole-source
/// re-derivation regardless of what window a caller happens to pass.
#[tokio::test]
async fn range_rebuild_re_derives_the_whole_ledger() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "2026-01-01", "gold", false);
        insert_event(
            &conn,
            1,
            "2026-01-02 08:00:00",
            "2026-01-02",
            "silver",
            true,
        );
    }
    run_steps(
        &db_path,
        &day("2026-01-01", "2026-01-03"),
        &cell(recipe(true)),
    )
    .await
    .expect("bootstrap run succeeds");

    run_rebuild(&db_path, REBUILD_MODEL_SQL, &cell(recipe(true)))
        .await
        .expect("range rebuild succeeds");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(
            &conn,
            "(SELECT customer_id, changed_at FROM main.customer_history__tombstones) EXCEPT ALL \
             (SELECT customer_id, changed_at FROM main.raw_customer_changes WHERE is_deleted)"
        ),
        0,
        "the ledger must equal the whole source's delete-flagged rows, not a range-restricted \
         subset"
    );
}

fn succession_source_info() -> SourceInfo {
    SourceInfo {
        path: PathBuf::from("/tmp/customer_changes.yml"),
        address_segments: vec!["sources".to_string(), "customer_changes".to_string()],
        columns: vec![
            SourceColumn {
                name: "customer_id".to_string(),
                data_type: DataType::Integer,
                nullable: false,
                description: None,
            },
            SourceColumn {
                name: "changed_at".to_string(),
                data_type: ts(false),
                nullable: false,
                description: None,
            },
        ],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: Some(TimeseriesConfig {
            event_time_column: "changed_at".to_string(),
            partition_column: "changed_at".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        mutation_profile: Some(SourceMutationProfile::from_kind(
            SourceMutationKind::AppendOnly,
        )),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

fn succession_metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        ..Default::default()
    }
}

const SUCCESSION_SQL: &str = "SELECT \
     customer_id, \
     changed_at, \
     name, \
     LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at \
     FROM smelt.sources.customer_changes";

/// Test 9: `state_downgraded_cell_is_not_dispatched` —
/// `StateAvailability::none()` resolves `None` (the cell downgrades to
/// `Technique::DeleteInsert`, no longer `SuccessionPatch`), while
/// `StateAvailability::all()` resolves `Some` for the SAME model — proving
/// the `None` above is the downgrade, not a resolver bug.
#[test]
fn state_downgraded_cell_is_not_dispatched() {
    let metadata = succession_metadata();
    let source_refs = vec![(
        "customer_changes".to_string(),
        Some(succession_source_info()),
    )];
    let source_infos = vec![succession_source_info()];

    let downgraded = resolve_live_succession_cell(
        SUCCESSION_SQL,
        "customer_history",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        &source_refs,
        &StateAvailability::none(),
        "main",
        "dev",
        &source_infos,
    )
    .expect("resolver does not error");
    assert!(
        downgraded.is_none(),
        "a state-downgraded cell (technique becomes DeleteInsert) must not dispatch"
    );

    let live = resolve_live_succession_cell(
        SUCCESSION_SQL,
        "customer_history",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        &source_refs,
        &StateAvailability::all(),
        "main",
        "dev",
        &source_infos,
    )
    .expect("resolver does not error")
    .expect("full availability must resolve a live succession cell");
    assert_eq!(live.recipe.key_cols, vec!["customer_id".to_string()]);
    assert_eq!(live.recipe.clock_col, "changed_at");
    assert_eq!(live.source_table, "main.sources_customer_changes");
    assert_eq!(live.partition_column, "changed_at");
}
