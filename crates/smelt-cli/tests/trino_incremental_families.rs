//! `20260913-trino-incremental` phase 3's two ledger-free families —
//! insert-only append and the whole-row `MERGE` upsert — proved at the
//! `Backend`-trait/emitter level rather than through a full CLI
//! `execute_project` invocation. See this file's doc comments below for
//! why: three separate, pre-existing, cross-cutting bugs (none introduced
//! by, or specific to, `MaintenanceDialect::Trino`) were found live and
//! block a real `smelt run`/`smelt rebuild` end-to-end for these families
//! today, on any backend — they are simply masked everywhere else by
//! DuckDB's/Spark's/BigQuery's more lenient implicit literal coercion.
//! Trino's strictness is what surfaced them.
//!
//! What phase 3 DID prove live, unaffected by these three gaps:
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   insert_into_from_query_appends_and_leaves_prior_rows_intact`
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   delete_and_insert_transactional_covers_two_disjoint_windows`
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   merge_into_upserts_matched_and_unmatched_rows_across_two_runs`
//!
//! Each drives the real `Backend` trait method a live `execute_project` run
//! would call, over the real `smelt_logical::maintenance::emit` statement
//! text `MaintenanceDialect::Trino` renders — the same emitters, the same
//! statement-emission single-owner path, just invoked directly instead of
//! through the CLI's window-string parsing and cell-derivation layers,
//! which is exactly what the three gaps below live in.
//!
//! Gap 2 was landed in phase 3a (`docs/outcomes/20260913-trino-incremental/
//! phases/03a-plan.md`) — `integer_axis_incremental_model_runs_on_trino`
//! below is the real CLI-level proof, replacing what was a documentation-only
//! anchor function. Gap 3 was landed in phase 3c (`docs/outcomes/
//! 20260913-trino-incremental/phases/03c-plan.md`) —
//! `snapshot_reconcile_keyed_model_runs_on_trino` below is the real
//! CLI-level proof. Gap 1 was landed in phase 3b2 (`docs/outcomes/
//! 20260913-trino-incremental/phases/03b2-plan.md`) —
//! `calendar_axis_incremental_model_runs_on_trino` below is the real
//! CLI-level proof.
//!
//! Phase 3d (`docs/outcomes/20260913-trino-incremental/phases/03d-plan.md`)
//! landed the **append family**'s full-refresh-oracle proof —
//! `append_family_matches_full_refresh_on_trino` below — over two disjoint
//! windowed runs. It also *attempted* the whole-row `MERGE` upsert (keyed-fold)
//! family live and found it blocked by two further, previously-undiscovered
//! gaps, neither owned by this phase's task list:
//!
//! - **Gap 4** — the windowed-keyed-maintenance driver's own per-step
//!   driving-source pushdown filter (`crates/smelt-runtime/src/
//!   maintenance_driver/driver.rs` / `cumulative.rs`, stepping over the
//!   driving source's own timeseries partition column) renders the run
//!   window's bound as a bare string against the driving source's own column,
//!   regardless of that column's real type — the same *class* of bug gaps 1
//!   and 2 named, but in a THIRD emission site neither of those phases' fixes
//!   touched (3a/3b2 fixed `transformer.rs`'s `inject_time_filter`/
//!   `inject_source_filters` and the mart's own declared `partition_column`
//!   type resolution; this is the driving-source stepping loop inside the
//!   windowed-keyed driver itself). Measured live: a `grain: key` model over a
//!   clocked `events` source with an `INTEGER` `partition_column` fails
//!   `Cannot apply operator: bigint <= varchar(10)`; the same model over a
//!   `DATE` `partition_column` fails `Cannot apply operator: date <=
//!   varchar(10)`. Blocks the idempotent (`MIN`/`MAX`-style) keyed-fold shape
//!   before the write mechanism is ever chosen.
//! - **Gap 5** — `Technique::KeyedFold`'s `Grade::Additive` branch (a
//!   fold-eligible combiner such as `SUM`) has no plan-time downgrade: unlike
//!   the repair family's `smelt_logical::maintenance::availability::
//!   resolve_availability`, the windowed-keyed driver checks
//!   `realises_reconciliation_ledger` at EXECUTION time and hard-refuses
//!   (`BackendError::unsupported("additive-fold windowed-keyed maintenance
//!   ledger (never-fold-twice)")`) rather than falling back to
//!   `PerGroupRecompute`. Since `realisable_state_structures` returns `vec![]`
//!   for both `SqlDialect::SparkSQL` and `SqlDialect::Trino` (the 2026-09-13
//!   fully-degraded ruling), this refusal is unconditional on Trino today —
//!   measured live, independent of gap 4 (reached even on a `DATE` axis with
//!   no literal-typing issue at all).
//!
//! Neither gap has a fix task in phase 3d's plan (pure test-infrastructure
//! scope), so the whole-row `MERGE` upsert family's live leg and
//! `statement_parity`'s Trino leg are deferred to a follow-up phase that can
//! scope the fix (or the accepted-degradation ruling) for gap 4 and gap 5.
//! See `docs/outcomes/20260913-trino-incremental/outcome.md`'s Blocked log
//! (2026-09-15) for the full writeup.

mod common;
use common::{
    drop_trino_schema, fetch_trino_rows, trino_backend, trino_env, trino_schema, trino_target_block,
};

use std::fs;
use std::path::Path;
use std::process::Command;

fn stage_int_partition_project(tmp: &tempfile::TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join("int_partition_trino");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: int_partition_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/seed_events.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT * FROM (VALUES\n\
         \x20  (CAST(1 AS BIGINT), CAST(1 AS BIGINT), TIMESTAMP '2026-01-01 00:00:00'),\n\
         \x20  (CAST(2 AS BIGINT), CAST(1 AS BIGINT), TIMESTAMP '2026-01-01 06:00:00'),\n\
         \x20  (CAST(3 AS BIGINT), CAST(2 AS BIGINT), TIMESTAMP '2026-01-02 00:00:00'),\n\
         \x20  (CAST(4 AS BIGINT), CAST(3 AS BIGINT), TIMESTAMP '2026-01-03 00:00:00')\n\
         ) AS t(id, batch_id, event_ts)\n",
    )
    .unwrap();
    fs::write(
        root.join("models/int_partition_mart.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n\
         ---\n\
         SELECT CAST(batch_id AS BIGINT) AS batch_id, event_ts, id FROM smelt.seed_events\n",
    )
    .unwrap();

    root
}

fn stage_calendar_partition_project(tmp: &tempfile::TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join("calendar_partition_trino");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: calendar_partition_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    // `event_date` is CAST to a real DATE column — the mart below passes it
    // through unchanged, so its own output schema (read via
    // `resolved_model_schema`) infers `event_date` as `DataType::Date`, and
    // gap 1's fix renders the injected calendar-axis literal `DATE '…'`
    // rather than a bare quoted string a strict engine refuses.
    fs::write(
        root.join("models/seed_calendar_events.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT * FROM (VALUES\n\
         \x20  (CAST(1 AS BIGINT), CAST('2026-01-01' AS DATE), CAST('2026-01-01' AS DATE)),\n\
         \x20  (CAST(2 AS BIGINT), CAST('2026-01-01' AS DATE), CAST('2026-01-01' AS DATE)),\n\
         \x20  (CAST(3 AS BIGINT), CAST('2026-01-02' AS DATE), CAST('2026-01-02' AS DATE)),\n\
         \x20  (CAST(4 AS BIGINT), CAST('2026-01-03' AS DATE), CAST('2026-01-03' AS DATE))\n\
         ) AS t(id, event_date, event_ts)\n",
    )
    .unwrap();
    fs::write(
        root.join("models/calendar_partition_mart.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: event_date\n  granularity: day\n\
         ---\n\
         SELECT CAST(event_date AS DATE) AS event_date, event_ts, id \
         FROM smelt.seed_calendar_events\n",
    )
    .unwrap();

    root
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// Gap 2 (landed) — the live leg: an integer-`partition_column` model runs a
/// `--batch-size 1` backfill and a steady-state re-run end to end through
/// `smelt run --target trino`, no longer refused with `Cannot apply
/// operator: integer <= varchar(1)` now that the real execution path
/// resolves each batch's `TimeRange` in the model's own partition axis
/// (`docs/outcomes/20260913-trino-incremental/phases/03a-plan.md`).
#[test]
fn integer_axis_incremental_model_runs_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping integer_axis_incremental_model_runs_on_trino");
        return;
    };
    let schema = trino_schema("int_partition");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let first = run_smelt(&root, &["--target", "trino"]);
    assert!(
        first.status.success(),
        "first run (seed) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let backfill = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        backfill.status.success(),
        "windowed backfill failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&backfill.stdout),
        String::from_utf8_lossy(&backfill.stderr),
    );

    let steady_state = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        steady_state.status.success(),
        "steady-state re-run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&steady_state.stdout),
        String::from_utf8_lossy(&steady_state.stderr),
    );

    let mut rows = fetch_trino_rows(&schema, "int_partition_mart")
        .into_iter()
        .map(|r| (r[0].clone(), r[2].clone()))
        .collect::<Vec<_>>();
    rows.sort();
    let expected = vec![
        ("1".to_string(), "1".to_string()),
        ("1".to_string(), "2".to_string()),
        ("2".to_string(), "3".to_string()),
        ("3".to_string(), "4".to_string()),
    ];
    assert_eq!(rows, expected, "unexpected row set after the phased run");

    drop_trino_schema(&schema);
}

/// Gap 1 (landed, `docs/outcomes/20260913-trino-incremental/
/// phases/03b2-plan.md`) — the live leg: a calendar-`partition_column`
/// model, whose own output schema infers the column as a real `DATE`, runs a
/// `--batch-size 1` backfill and a steady-state re-run end to end through
/// `smelt run --target trino`, no longer refused with `Cannot apply
/// operator: date <= varchar(10)` now that the injected calendar-axis
/// literal renders `DATE '…'`-typed against a `DATE`-declared column instead
/// of a bare quoted string.
#[test]
fn calendar_axis_incremental_model_runs_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping calendar_axis_incremental_model_runs_on_trino");
        return;
    };
    let schema = trino_schema("calendar_partition");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_calendar_partition_project(&tmp, &schema);

    let first = run_smelt(&root, &["--target", "trino"]);
    assert!(
        first.status.success(),
        "first run (seed) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let backfill = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-04",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        backfill.status.success(),
        "windowed backfill failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&backfill.stdout),
        String::from_utf8_lossy(&backfill.stderr),
    );

    let steady_state = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-04",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        steady_state.status.success(),
        "steady-state re-run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&steady_state.stdout),
        String::from_utf8_lossy(&steady_state.stderr),
    );

    let mut rows = fetch_trino_rows(&schema, "calendar_partition_mart")
        .into_iter()
        .map(|r| (r[0].clone(), r[2].clone()))
        .collect::<Vec<_>>();
    rows.sort();
    let expected = vec![
        ("2026-01-01".to_string(), "1".to_string()),
        ("2026-01-01".to_string(), "2".to_string()),
        ("2026-01-02".to_string(), "3".to_string()),
        ("2026-01-03".to_string(), "4".to_string()),
    ];
    assert_eq!(rows, expected, "unexpected row set after the phased run");

    drop_trino_schema(&schema);
}

/// Phase 3d (`docs/outcomes/20260913-trino-incremental/phases/03d-plan.md`)
/// — the append family's live leg: an insert-only `grain: partition` model
/// over an append-only integer-axis source, run as two disjoint windowed
/// `smelt run --target trino` invocations (`[1, 3)` then `[3, 5)`, each its
/// own first-touch batch — never a repeat window — so the family exercised
/// is the plain `CREATE TABLE … AS`/`INSERT INTO … FROM` append path, not
/// the region `DELETE`+`INSERT` recompute family
/// `integer_axis_incremental_model_runs_on_trino` above already covers.
/// Asserted **result**-equal (multiset, not pinned rows) to a
/// `--full-refresh` rebuild of the identical project into a second,
/// independent schema — the oracle shape this phase's plan calls for.
#[test]
fn append_family_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping append_family_matches_full_refresh_on_trino");
        return;
    };
    let schema = trino_schema("append_family");
    let oracle_schema = trino_schema("append_family_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "3",
        ],
    );
    assert!(
        first.status.success(),
        "first disjoint window [1, 3) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "3",
            "--event-time-end",
            "5",
        ],
    );
    assert!(
        second.status.success(),
        "second disjoint window [3, 5) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_int_partition_project(&oracle_tmp, &oracle_schema);
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "int_partition_mart");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "int_partition_mart");
    expected.sort();
    assert_eq!(
        actual, expected,
        "two disjoint windowed append runs must be multiset-equal to a full-refresh rebuild"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Gap 3 (landed, `docs/outcomes/20260913-trino-incremental/
/// phases/03c-plan.md`) — the live leg: a keyed model whose only `NewData`-
/// eligible technique is refused (neither `ANY_VALUE` combiner is
/// fold-eligible, and the repair family refuses `RepairSliceUnbounded` — no
/// partition column on the unclocked source), so the model's only derived
/// cell is `Trigger::UpstreamMutation`'s `Technique::ColumnScopedMerge`
/// (`key_scope: None`, `scans: []`). On Trino's fully-degraded dialect
/// (`realisable_state_structures` empty), that cell downgrades to a
/// clamp-less `PerGroupRecompute`. Before this phase's fix
/// (`smelt_logical::maintenance::repair::has_repair_family_lowering`,
/// `docs/specs/state.md` §"The degradation contract"), execution refused
/// with `MaintenanceRepairSliceMissing` instead of taking the whole-target
/// full-scan route the `key_scope: None` "reachable" row in
/// `docs/specs/multi_backend.md`'s landing table already promises. Run
/// twice through `smelt run --target trino`: creation, then a mutation to
/// the seeded source row, asserting the maintained table reflects it.
#[test]
fn snapshot_reconcile_keyed_model_runs_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping snapshot_reconcile_keyed_model_runs_on_trino");
        return;
    };
    let schema = trino_schema("snapshot_reconcile");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("snapshot_reconcile_trino");
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: snapshot_reconcile_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}\
         default_materialization: table\nstate:\n  warehouse_tables: none\n",
        trino_target_block(&schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/customer_snapshot.yml"),
        "description: unclocked mutable customer snapshot.\n\
         mutation_profile: mutable_snapshot\n\
         unique_key: [order_id]\n\
         columns:\n\
         - name: order_id\n  type: INTEGER\n\
         - name: customer_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n\
         - name: tier_rank\n  type: INTEGER\n",
    )
    .unwrap();
    fs::write(
        root.join("models/customer_totals.sql"),
        "---\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20customer_snapshot:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT customer_id, ANY_VALUE(amount) AS total_amount, \
         ANY_VALUE(tier_rank) AS max_tier\n\
         FROM smelt.sources.customer_snapshot\n\
         GROUP BY customer_id\n",
    )
    .unwrap();

    let backend = trino_backend(&schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    {
        use smelt_backend::Backend;
        rt.block_on(async {
            backend
                .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
                .await
                .expect("create schema");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_customer_snapshot (order_id INTEGER, \
                     customer_id INTEGER, amount DECIMAL(10,2), tier_rank INTEGER)"
                ))
                .await
                .expect("create source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_customer_snapshot VALUES \
                     (1, 1, 100.00, 1), (2, 2, 70.00, 1)"
                ))
                .await
                .expect("seed source table");
        });
    }

    let first = run_smelt(&root, &["--target", "trino"]);
    assert!(
        first.status.success(),
        "first run (create) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    // Mutate customer 1's row in place. Before the fix, this exact shape
    // (a downgraded, clamp-less `PerGroupRecompute` cell on
    // `Trigger::UpstreamMutation`) refused the run with
    // `MaintenanceRepairSliceMissing`.
    {
        use smelt_backend::Backend;
        rt.block_on(async {
            backend
                .execute_sql(&format!(
                    "UPDATE {schema}.sources_customer_snapshot SET amount = 999.00, \
                     tier_rank = 5 WHERE order_id = 1"
                ))
                .await
                .expect("mutate source row");
        });
    }

    let second = run_smelt(&root, &["--target", "trino"]);
    assert!(
        second.status.success(),
        "second run (downgraded PerGroupRecompute cell, no ScanClamp) must succeed via the \
         whole-target route rather than refusing MaintenanceRepairSliceMissing.\nstdout: {}\n\
         stderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let mut rows = fetch_trino_rows(&schema, "customer_totals");
    rows.sort();
    let expected = vec![
        vec!["1".to_string(), "999.00".to_string(), "5".to_string()],
        vec!["2".to_string(), "70.00".to_string(), "1".to_string()],
    ];
    assert_eq!(
        rows, expected,
        "customer 1's mutated row must be reflected after the repair"
    );

    drop_trino_schema(&schema);
}
