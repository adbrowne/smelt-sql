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
//! `statement_parity`'s Trino leg were deferred at the time. See
//! `docs/outcomes/20260913-trino-incremental/outcome.md`'s Blocked log
//! (2026-09-15) for the full writeup of both gaps as originally measured.
//!
//! **Both gaps are now closed.** Gap 4 was fixed in phase 3f (`docs/
//! outcomes/20260913-trino-incremental/phases/03f-plan.md`): every
//! partition literal the windowed-keyed maintenance driver emits — the
//! driving-source pushdown filter included — now goes through the single
//! typed literal-renderer phase 3b2 introduced. Gap 5 was fixed in phase 3g
//! (`docs/outcomes/20260913-trino-incremental/phases/03g-plan.md`):
//! `Technique::KeyedFold` now resolves its state requirement **by grade** at
//! plan time (`PlanCell::fold_grade`) — an idempotent fold (`MIN`/`MAX`-
//! style) needs no structure and keeps the `MERGE` route; an additive fold
//! (`SUM`-style, or a decomposed `AVG`/`STDDEV_*`/`VAR_*`) has no realisable
//! reconciliation ledger on a fully-degraded backend and downgrades,
//! unconditionally and explain-visibly, to the same whole-target
//! drop+recreate a keyless `--full-refresh` already uses.
//!
//! Phase 3h (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`)
//! re-attempted the whole-row `MERGE` upsert family live on both fronts:
//! - `whole_row_merge_upsert_matches_full_refresh_on_trino` — the idempotent
//!   (`MIN`-combiner) shape, run over two disjoint windows (the second both
//!   revising an existing key and introducing a new one), asserted
//!   multiset-equal to a `--full-refresh` oracle.
//! - `whole_row_merge_upsert_writes_through_the_merge_route_on_trino` — the
//!   same shape's `--json` plan report shows the cell resolving to
//!   `KeyedFold` with no `state_downgrade`, so the multiset-equal proof
//!   above cannot pass vacuously via the degraded rebuild path.
//! - `additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino`
//!   — the same fixture with a `SUM` combiner: the cell is recorded
//!   downgraded (`--json`'s `state_downgrade.original == "KeyedFold"`,
//!   `technique == "PerGroupRecompute"`), and the maintained table is still
//!   multiset-equal to the full-refresh oracle after each of two windows —
//!   a downgraded cell is asserted oracle-equal, not exempted, on the live
//!   tier.
//!
//! `statement_parity`'s Trino leg (byte-identity between the executed
//! `MERGE` and a direct `emit_keyed_fold_suppressed` call) lives in
//! `crates/smelt-runtime/tests/statement_parity/trino.rs`, not here — that
//! suite already owns the emitter-parity proof for every other family.
//!
//! Phase 4 (`docs/outcomes/20260913-trino-incremental/phases/04-plan.md`)
//! proves the emulated delete-and-insert window family (`Technique::
//! DeleteInsert`, Trino's `INSERT OVERWRITE` emulation) end to end, over
//! three behavioural properties:
//! `delete_insert_window_replaces_only_its_own_rows_on_trino` (over
//! `stage_delete_insert_mutation_project`, a declared-source mart so a raw
//! source-row mutation between runs survives — `stage_int_partition_project`'s
//! `seed_events` is a plain first-class model smelt rebuilds from its
//! static `VALUES` body on every run, which would silently erase the
//! mutation): a mutated source row surfaces exactly once, and neighbouring
//! batches stay byte-identical — no wider, no narrower.
//! `delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino`
//! (over `int_partition_mart`/`stage_int_partition_project`) — a later
//! window applied before an earlier one still matches a `--full-refresh`
//! oracle — and
//! `delete_insert_repeated_window_is_idempotent_on_trino` (re-applying the
//! same window twice with no source change reproduces the same rows
//! exactly). `statement_parity::trino::delete_insert_parity_on_trino`
//! proves the fourth: the executed statements are byte-identical to a
//! direct `emit_delete_insert` call.

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

// =============================================================================
// Phase 3h: the whole-row `MERGE` upsert (keyed-fold) family, live on Trino
// (gaps 4 and 5 closed in phases 3f/3g — see this file's module doc).
// =============================================================================

/// A `refresh: incremental` / `grain: key` model over a clocked, append-only
/// `events` source (`event_date` a real `DATE` `timeseries.partition_column`).
/// `combiner` parameterises the fold: `MIN`/`MAX` grades `Grade::Idempotent`
/// (stays on the `MERGE` route); `SUM` grades `Grade::Additive` (downgrades
/// on Trino's fully-degraded dialect, per gap 5's fix). `allow_full_scan` is
/// declared unconditionally — the additive downgrade's whole-target rebuild
/// reads the source unwindowed regardless of which combiner this particular
/// project uses.
fn stage_keyed_fold_project(
    tmp: &tempfile::TempDir,
    schema: &str,
    combiner: &str,
) -> std::path::PathBuf {
    let root = tmp.path().join(format!("keyed_fold_trino_{schema}"));
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: keyed_fold_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/events.yml"),
        "description: Clocked per-device events.\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: event_date\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
    )
    .unwrap();

    fs::write(
        root.join("models/device_agg.sql"),
        format!(
            "---\n\
             materialization: table\n\
             refresh: incremental\n\
             grain: key\n\
             maintenance:\n\
             \x20\x20scan_bounds:\n\
             \x20\x20\x20\x20per_source:\n\
             \x20\x20\x20\x20\x20\x20events:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
             ---\n\
             SELECT device_id, {combiner}(amount) AS agg_amount \
             FROM smelt.sources.events GROUP BY 1\n"
        ),
    )
    .unwrap();

    root
}

/// Seed `schema.sources_events` (create schema, create table, insert `rows`)
/// on the live Trino tier.
fn seed_trino_events(schema: &str, rows: &[(i64, &str, f64)]) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let values: Vec<String> = rows
        .iter()
        .map(|(id, date, amount)| format!("({id}, DATE '{date}', {amount})"))
        .collect();
    rt.block_on(async {
        backend
            .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .await
            .expect("create schema");
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_events (device_id INTEGER, event_date DATE, \
                 amount DOUBLE)"
            ))
            .await
            .expect("create source table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_events VALUES {}",
                values.join(", ")
            ))
            .await
            .expect("seed source table");
    });
}

/// Insert additional rows into an already-seeded `schema.sources_events`.
fn insert_trino_events(schema: &str, rows: &[(i64, &str, f64)]) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let values: Vec<String> = rows
        .iter()
        .map(|(id, date, amount)| format!("({id}, DATE '{date}', {amount})"))
        .collect();
    rt.block_on(async {
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_events VALUES {}",
                values.join(", ")
            ))
            .await
            .expect("insert additional rows");
    });
}

fn explain_json(project_dir: &Path, model_name: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg(model_name)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain`: {e}"));
    assert!(
        output.status.success(),
        "smelt explain --json failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("explain --json must parse: {e}\n{stdout}"))
}

/// Test 1 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the idempotent (`MIN`-combiner) whole-row `MERGE` upsert family, run over
/// two disjoint windows where the second both revises an existing key
/// (device 1's amount drops from 50 to 5, a matched-arm update under `MIN`)
/// and introduces a new key (device 3, a not-matched-arm insert), asserted
/// multiset-equal to a `--full-refresh` rebuild of the same, fully-seeded
/// source into an independent schema.
#[test]
fn whole_row_merge_upsert_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping whole_row_merge_upsert_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_merge");
    let oracle_schema = trino_schema("keyed_merge_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "MIN");
    seed_trino_events(&schema, &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)]);

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-02",
        ],
    );
    assert!(
        first.status.success(),
        "first window (create) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    insert_trino_events(&schema, &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)]);

    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-02",
            "--event-time-end",
            "2026-01-03",
        ],
    );
    assert!(
        second.status.success(),
        "second window (merge) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_keyed_fold_project(&oracle_tmp, &oracle_schema, "MIN");
    seed_trino_events(
        &oracle_schema,
        &[
            (1, "2026-01-01", 50.0),
            (2, "2026-01-01", 20.0),
            (1, "2026-01-02", 5.0),
            (3, "2026-01-02", 30.0),
        ],
    );
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "device_agg");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "device_agg");
    expected.sort();
    assert_eq!(
        actual, expected,
        "two disjoint windowed keyed-fold merge runs must be multiset-equal to a full-refresh \
         rebuild"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Test 2 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the same idempotent shape's `smelt explain --json` plan report shows the
/// cell resolving to `KeyedFold` with no `state_downgrade` — proving the
/// oracle-equality test above cannot pass vacuously via the degraded
/// whole-target rebuild path. Offline: `smelt explain` never opens a live
/// connection.
#[test]
fn whole_row_merge_upsert_writes_through_the_merge_route_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             whole_row_merge_upsert_writes_through_the_merge_route_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_merge_explain");
    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "MIN");

    let json = explain_json(&root, "device_agg");
    let cells = json["cells"].as_array().expect("cells array");
    let cell = cells
        .iter()
        .find(|c| c["technique"] == "KeyedFold")
        .unwrap_or_else(|| panic!("expected a KeyedFold cell: {json}"));
    assert!(
        cell.get("state_downgrade").is_none(),
        "an idempotent (MIN) combiner's keyed fold must not be downgraded on Trino: {json}"
    );
}

/// Test 3 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the same fixture with a `SUM` combiner — `Grade::Additive`, no
/// realisable reconciliation ledger on Trino's fully-degraded dialect. The
/// cell must be recorded downgraded (`state_downgrade.original ==
/// "KeyedFold"`, `technique == "PerGroupRecompute"`), and the maintained
/// table must still be multiset-equal to the full-refresh oracle after
/// EACH of two windows — a downgraded cell is asserted oracle-equal, not
/// exempted.
#[test]
fn additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_fold_downgrade");
    let oracle_schema = trino_schema("keyed_fold_downgrade_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "SUM");
    seed_trino_events(&schema, &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)]);

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-02",
        ],
    );
    assert!(
        first.status.success(),
        "first window (downgraded whole-target rebuild) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let json = explain_json(&root, "device_agg");
    let cells = json["cells"].as_array().expect("cells array");
    let downgraded = cells
        .iter()
        .find(|c| c.get("state_downgrade").is_some())
        .unwrap_or_else(|| panic!("expected a cell carrying state_downgrade: {json}"));
    assert_eq!(downgraded["state_downgrade"]["original"], "KeyedFold");
    assert_eq!(downgraded["technique"], "PerGroupRecompute");

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_keyed_fold_project(&oracle_tmp, &oracle_schema, "SUM");
    seed_trino_events(
        &oracle_schema,
        &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)],
    );
    let oracle_run_1 = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run_1.status.success(),
        "oracle full-refresh (window 1) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run_1.stdout),
        String::from_utf8_lossy(&oracle_run_1.stderr),
    );

    let mut actual_1 = fetch_trino_rows(&schema, "device_agg");
    actual_1.sort();
    let mut expected_1 = fetch_trino_rows(&oracle_schema, "device_agg");
    expected_1.sort();
    assert_eq!(
        actual_1, expected_1,
        "after window 1, the downgraded cell must already match the full-refresh oracle"
    );

    insert_trino_events(&schema, &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)]);
    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-02",
            "--event-time-end",
            "2026-01-03",
        ],
    );
    assert!(
        second.status.success(),
        "second window (downgraded whole-target rebuild) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    insert_trino_events(
        &oracle_schema,
        &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)],
    );
    let oracle_run_2 = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run_2.status.success(),
        "oracle full-refresh (window 2) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run_2.stdout),
        String::from_utf8_lossy(&oracle_run_2.stderr),
    );

    let mut actual_2 = fetch_trino_rows(&schema, "device_agg");
    actual_2.sort();
    let mut expected_2 = fetch_trino_rows(&oracle_schema, "device_agg");
    expected_2.sort();
    assert_eq!(
        actual_2, expected_2,
        "after window 2, the downgraded cell must still match the full-refresh oracle"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Runs one raw SQL statement against `schema` on the live Trino tier —
/// used below to mutate a declared source's underlying table directly
/// between `smelt run` invocations, the same bridging pattern as
/// `seed_trino_events`/`insert_trino_events`.
fn run_trino_sql(schema: &str, sql: &str) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        backend
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("run_trino_sql({sql:?}) failed: {e}"));
    });
}

/// A `grain: partition`/`DeleteInsert` mart over a **declared external**
/// source (`models/sources/events.yml`) rather than a first-class inline
/// model: unlike `stage_int_partition_project`'s `seed_events` (a plain
/// `materialization: table` model smelt itself rebuilds from its static
/// `VALUES` body on every run, silently undoing any raw mutation between
/// `smelt run` invocations), a declared source's backing table is never
/// written by smelt — only read — so a raw `DELETE`+`INSERT` against it
/// between runs stays in place for the next run to observe.
fn stage_delete_insert_mutation_project(
    tmp: &tempfile::TempDir,
    schema: &str,
) -> std::path::PathBuf {
    let root = tmp.path().join("delete_insert_mutation_trino");
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: delete_insert_mutation_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/events.yml"),
        "description: Batched events source.\n\
         columns:\n\
         \x20\x20- name: id\n\
         \x20\x20\x20\x20type: BIGINT\n\
         \x20\x20- name: batch_id\n\
         \x20\x20\x20\x20type: BIGINT\n\
         \x20\x20- name: event_ts\n\
         \x20\x20\x20\x20type: TIMESTAMP\n\
         timeseries:\n\
         \x20\x20event_time_column: event_ts\n\
         \x20\x20partition_column: batch_id\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
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
         maintenance:\n\
         \x20 scan_bounds:\n    per_source:\n      events:\n        allow_full_scan: true\n\
         ---\n\
         SELECT id, batch_id, event_ts FROM smelt.sources.events\n",
    )
    .unwrap();

    root
}

/// Seeds the declared `events` source's backing table directly on the live
/// tier — smelt never writes this table itself, so this is the only writer.
fn seed_delete_insert_source(schema: &str, rows: &[(i64, i64, &str)]) {
    let values: Vec<String> = rows
        .iter()
        .map(|(id, batch_id, ts)| {
            format!("(CAST({id} AS BIGINT), CAST({batch_id} AS BIGINT), TIMESTAMP '{ts}')")
        })
        .collect();
    run_trino_sql(schema, &format!("CREATE SCHEMA IF NOT EXISTS {schema}"));
    run_trino_sql(
        schema,
        &format!(
            "CREATE TABLE {schema}.sources_events (id BIGINT, batch_id BIGINT, event_ts TIMESTAMP)"
        ),
    );
    run_trino_sql(
        schema,
        &format!(
            "INSERT INTO {schema}.sources_events VALUES {}",
            values.join(", ")
        ),
    );
}

/// Phase 4 (`docs/outcomes/20260913-trino-incremental/phases/04-plan.md`),
/// criterion 4 — the emulated delete-and-insert window's `DELETE` covers
/// exactly the range the `INSERT` writes: no wider (data loss) and no
/// narrower (duplicates). The declared `events` source is seeded across
/// three batches (`batch_id` 1, 2, 3); run 1 materialises all three via
/// window `[1, 4)`, the source row driving batch 2 (`id = 3`) is then
/// mutated directly on the live tier, and run 2 re-runs only window
/// `[2, 3)`. Batch 1 (`id` 1, 2) and batch 3 (`id` 4) must be byte-identical
/// to before (proving the `DELETE` did not widen into either neighbour),
/// and batch 2's row must reflect the mutation exactly once (proving the
/// `INSERT` did not narrow and silently drop it).
#[test]
fn delete_insert_window_replaces_only_its_own_rows_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_window_replaces_only_its_own_rows_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_window");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_delete_insert_mutation_project(&tmp, &schema);
    seed_delete_insert_source(
        &schema,
        &[
            (1, 1, "2026-01-01 00:00:00"),
            (2, 1, "2026-01-01 06:00:00"),
            (3, 2, "2026-01-02 00:00:00"),
            (4, 3, "2026-01-03 00:00:00"),
        ],
    );

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
        ],
    );
    assert!(
        first.status.success(),
        "first run (materialise all batches) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let before = fetch_trino_rows(&schema, "int_partition_mart");
    let batch1_before: Vec<_> = before.iter().filter(|r| r[1] == "1").cloned().collect();
    let batch3_before: Vec<_> = before.iter().filter(|r| r[1] == "3").cloned().collect();
    assert_eq!(batch1_before.len(), 2, "batch 1 seeds two rows: {before:?}");
    assert_eq!(batch3_before.len(), 1, "batch 3 seeds one row: {before:?}");

    // Mutate only the source row driving batch 2's single row (`id = 3`):
    // move its event timestamp within the same day, so the mutation is
    // visible in the mart without changing which batch it belongs to.
    run_trino_sql(
        &schema,
        &format!("DELETE FROM {schema}.sources_events WHERE id = CAST(3 AS BIGINT)"),
    );
    run_trino_sql(
        &schema,
        &format!(
            "INSERT INTO {schema}.sources_events VALUES \
             (CAST(3 AS BIGINT), CAST(2 AS BIGINT), TIMESTAMP '2026-01-02 12:00:00')"
        ),
    );

    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2",
            "--event-time-end",
            "3",
        ],
    );
    assert!(
        second.status.success(),
        "second run (narrow window over batch 2 only) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let after = fetch_trino_rows(&schema, "int_partition_mart");
    let batch1_after: Vec<_> = after.iter().filter(|r| r[1] == "1").cloned().collect();
    let batch2_after: Vec<_> = after.iter().filter(|r| r[1] == "2").cloned().collect();
    let batch3_after: Vec<_> = after.iter().filter(|r| r[1] == "3").cloned().collect();

    assert_eq!(
        batch1_after, batch1_before,
        "batch 1 must be untouched — the narrower [2, 3) DELETE must not widen into it"
    );
    assert_eq!(
        batch3_after, batch3_before,
        "batch 3 must be untouched — the narrower [2, 3) DELETE must not widen into it either"
    );
    assert_eq!(
        batch2_after.len(),
        1,
        "batch 2 must have exactly one row after the re-run — no duplication: {after:?}"
    );
    assert!(
        batch2_after[0][2].contains("12:00:00"),
        "batch 2's row must reflect the mutated event_ts exactly once: {after:?}"
    );

    drop_trino_schema(&schema);
}

/// Phase 4, criterion 4 — applied out of order. `int_partition_mart` is run
/// with its later window `[3, 5)` first (the first-run `CREATE TABLE … AS`
/// arm, materialising only batch 3), then its earlier window `[1, 3)`
/// (materialising batches 1 and 2 through the `DELETE`+`INSERT` recompute
/// arm). The result must be multiset-equal to a `--full-refresh` oracle of
/// the identical project in a separate schema — exact coverage holds
/// regardless of which order the windows arrive in.
#[test]
fn delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_out_of_order");
    let oracle_schema = trino_schema("delete_insert_out_of_order_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let later_window = run_smelt(
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
        later_window.status.success(),
        "later window [3, 5) (applied first) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&later_window.stdout),
        String::from_utf8_lossy(&later_window.stderr),
    );

    let earlier_window = run_smelt(
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
        earlier_window.status.success(),
        "earlier window [1, 3) (applied second) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&earlier_window.stdout),
        String::from_utf8_lossy(&earlier_window.stderr),
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
        "windows applied out of order must still match a full-refresh oracle"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Phase 4, criterion 4 — repeated application is idempotent. `int_partition_
/// mart`'s window `[1, 3)` is applied twice with no source change in
/// between; the second application's `DELETE`+`INSERT` must reproduce
/// exactly the same rows as the first, with no duplicate rows and no loss.
#[test]
fn delete_insert_repeated_window_is_idempotent_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_repeated_window_is_idempotent_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_repeated");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let args = [
        "--target",
        "trino",
        "--event-time-start",
        "1",
        "--event-time-end",
        "3",
    ];

    let first = run_smelt(&root, &args);
    assert!(
        first.status.success(),
        "first application of [1, 3) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );
    let mut once = fetch_trino_rows(&schema, "int_partition_mart");
    once.sort();

    let second = run_smelt(&root, &args);
    assert!(
        second.status.success(),
        "repeated application of [1, 3) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );
    let mut twice = fetch_trino_rows(&schema, "int_partition_mart");
    twice.sort();

    assert_eq!(
        once, twice,
        "re-running the identical window must be idempotent: no duplicate rows, no loss"
    );
    assert_eq!(
        twice.len(),
        3,
        "window [1, 3) covers batches 1 and 2: two rows plus one row: {twice:?}"
    );

    drop_trino_schema(&schema);
}
