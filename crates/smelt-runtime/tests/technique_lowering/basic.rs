use super::*;

#[test]
fn unadmitted_cell_never_lowers_targeted_write() {
    let plan = refused_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // Pinning `rederive_columns` names a cell the plan never admitted — a
    // pin bypasses the cost model, never admission
    // (`incremental_models.md` §"Per-cell admission"). This must refuse at
    // plan-resolution time, not silently execute a targeted write with an
    // unbounded footprint.
    let err = resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), true)
        .expect_err("a pin naming an unadmitted cell must refuse, never lower a targeted write");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic: {err}"
    );

    // Without a pin, the safe default is region-recompute — no error, and
    // critically NOT `ColumnScopedMerge` (there is no runtime fallback that
    // fabricates a targeted write the plan never admitted).
    let resolved = resolve_cell_technique(&plan, &trigger, None, true)
        .expect("no pin + unadmitted cell must fall back safely, not error");
    assert_eq!(resolved, ResolvedTechnique::RegionRecompute);
}

#[test]
fn backend_capability_gap_is_indistinguishable_from_plan_refusal() {
    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // The plan admits the cell, but the backend cannot execute a
    // column-scoped MERGE at all: this must behave exactly like an
    // unadmitted cell, never a runtime surprise after the plan already
    // chose the technique.
    let resolved = resolve_cell_technique(&plan, &trigger, None, false)
        .expect("capability gap without a pin falls back safely");
    assert_eq!(resolved, ResolvedTechnique::RegionRecompute);

    let err = resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), false)
        .expect_err("a pin naming a capability-gapped backend must refuse, not silently downgrade");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic: {err}"
    );
}

#[test]
fn admitted_cell_on_capable_backend_resolves_column_scoped_merge() {
    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let resolved = resolve_cell_technique(&plan, &trigger, None, true)
        .expect("admitted cell + capable backend resolves cleanly");
    assert_eq!(resolved, ResolvedTechnique::ColumnScopedMerge);

    // The hard pin agrees — pinning a technique the plan DID admit is not
    // an override, just an explicit restatement of what the cost model
    // would already have chosen.
    let pinned =
        resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), true)
            .expect("pinning an admitted, runnable technique must succeed");
    assert_eq!(pinned, ResolvedTechnique::ColumnScopedMerge);
}

/// End-to-end: a fact table `events_enriched(d, user_id, val, tier)` is
/// enriched from a mutable dimension `users(user_id, tier)`. A dimension
/// mutation is column-scoped MERGEd into just the `{tier}` group via
/// `execute_column_scoped_merge`, and the result matches a full-refresh
/// oracle re-joining the CURRENT dimension contents — the G-05/EX-13/EX-24
/// enrichment shape's equivalence leg (`smelt-maintenance-testkit`'s
/// `join_enrichment_mutable_dimension`), exercised directly against
/// `maintenance_driver`'s new physical primitive rather than through
/// `execute_project` (forward propagation — deciding *when* a dimension
/// mutation should trigger this cell — is MP15's job; this phase makes the
/// mechanism itself live and callable).
#[tokio::test]
async fn column_scoped_merge_matches_full_refresh_after_dimension_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (d DATE, user_id BIGINT, val DOUBLE, tier VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             (DATE '2024-01-01', 1, 10.0, 'bronze'), \
             (DATE '2024-01-01', 2, 20.0, 'silver')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
        .await
        .expect("seed dim table (user_id=1 mutated bronze -> gold)");

    assert!(
        backend.capabilities().supports_column_scoped_merge,
        "DuckDB backend must advertise column-scoped MERGE capability"
    );

    // The re-derivation batch for the `{tier}` group: DuckDB's `merge_into`
    // issues `UPDATE SET *`, which requires the source projection to carry
    // the FULL target row (a column-count mismatch is a hard backend
    // error, not a silent by-name subset) — so `val` passes through
    // unchanged from the existing target row (`e.val`) while `tier`
    // re-derives from the CURRENT dimension contents (`u.tier`). This is
    // what keeps the merge column-scoped in *effect*: only `tier`'s value
    // actually changes. The horizon clamp reuses the target's own `d`
    // column as the conversion-timestamp axis.
    let dimension_batch_sql = "SELECT e.d, e.user_id, e.val, u.tier \
         FROM main.events_enriched e JOIN main.sources_users u ON e.user_id = u.user_id";

    let contribution = ContributionVerdict::Monotone;
    let bound = BoundResult::Bounded {
        source_partition_col: "d".to_string(),
        before: Seconds::ZERO,
        after: Seconds::hours(24),
    };

    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let resolved = resolve_cell_technique(
        &plan,
        &trigger,
        None,
        backend.capabilities().supports_column_scoped_merge,
    )
    .expect("admitted cell resolves");
    assert_eq!(resolved, ResolvedTechnique::ColumnScopedMerge);

    execute_column_scoped_merge(
        &backend,
        "main",
        "events_enriched",
        &["user_id".to_string()],
        &contribution,
        &bound,
        "d",
        "2024-01-01 12:00:00",
        dimension_batch_sql,
        &[],
        &unconditional(),
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("column-scoped merge must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");

    let maintained_tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained tier for user 1");
    assert_eq!(
        maintained_tier_1, "gold",
        "column-scoped MERGE must pick up the mutated dimension value"
    );

    // `val` must be untouched — the merge is scoped to `{tier}` IN EFFECT,
    // even though the physical `merge_into` primitive issues an
    // `UPDATE SET *` over the full row: `dimension_batch_sql` carries `val`
    // through unchanged from the existing target row, so its value is a
    // no-op write, while `tier` re-derives from the CURRENT dimension.
    let val_1: f64 = conn
        .query_row(
            "SELECT val FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read untouched val for user 1");
    assert_eq!(
        val_1, 10.0,
        "column-scoped MERGE must not touch columns outside its group"
    );

    // Full-refresh oracle: re-join the CURRENT dimension contents from
    // scratch — no smelt compilation, no derived filter.
    let full_refresh_tier_1: String = conn
        .query_row(
            "SELECT u.tier FROM main.sources_users u WHERE u.user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("oracle read");
    assert_eq!(
        maintained_tier_1, full_refresh_tier_1,
        "column-scoped MERGE result must match the full-refresh oracle"
    );

    let unaffected_tier_2: String = conn
        .query_row(
            "SELECT tier FROM main.events_enriched WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read unaffected row");
    assert_eq!(
        unaffected_tier_2, "silver",
        "an unmutated dimension row's enrichment must be unchanged (still matches its own \
         full-refresh oracle value)"
    );
}

/// Sibling of `column_scoped_merge_matches_full_refresh_after_dimension_
/// mutation` above, proving the mutation-happened discrimination gate
/// (`smelt_runtime::mutation_probe`, `docs/specs/incremental_models.md`
/// §"When a mutation cell dispatches") actually skips the MERGE when the
/// mutated dimension is unchanged since the last recorded baseline: run once
/// (records the baseline), re-run with the dimension untouched → the gate
/// reports `NoOp` and the merge does not execute (result still equal to the
/// full-refresh oracle, since nothing needed to change), then mutate the
/// dimension and re-run → the gate reports `Dispatch` again.
#[tokio::test]
async fn column_scoped_merge_skipped_when_dimension_unmutated() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (d DATE, user_id BIGINT, val DOUBLE, tier VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             (DATE '2024-01-01', 1, 10.0, 'bronze'), \
             (DATE '2024-01-01', 2, 20.0, 'silver')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed dim table (unmutated — matches events_enriched already)");

    let digest_columns = vec!["user_id".to_string(), "tier".to_string()];
    let dialect = smelt_backend::MaintenanceDialect::DuckDb;

    // First gate call: no recorded baseline, so it always dispatches.
    let (verdict1, baseline1) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "events_enriched",
        "users",
        "main.sources_users",
        &digest_columns,
        dialect,
        None,
    )
    .await
    .expect("gate must succeed");
    assert_eq!(
        verdict1,
        smelt_runtime::mutation_probe::MutationVerdict::Dispatch
    );

    // Second gate call against the JUST-recorded baseline, dimension still
    // unchanged: must report NoOp — the cell is a no-op this run.
    let (verdict2, _baseline2) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "events_enriched",
        "users",
        "main.sources_users",
        &digest_columns,
        dialect,
        Some(&baseline1),
    )
    .await
    .expect("gate must succeed");
    assert_eq!(
        verdict2,
        smelt_runtime::mutation_probe::MutationVerdict::NoOp,
        "an unmutated dimension must not re-dispatch the column-scoped merge"
    );

    // The dimension is now genuinely mutated (user 1: bronze -> gold).
    backend
        .execute_sql("UPDATE main.sources_users SET tier = 'gold' WHERE user_id = 1")
        .await
        .expect("mutate dimension");

    let (verdict3, baseline3) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "events_enriched",
        "users",
        "main.sources_users",
        &digest_columns,
        dialect,
        Some(&baseline1),
    )
    .await
    .expect("gate must succeed");
    assert_eq!(
        verdict3,
        smelt_runtime::mutation_probe::MutationVerdict::Dispatch,
        "a genuinely mutated dimension must dispatch again"
    );
    assert_ne!(
        baseline3.recorded_fingerprint,
        baseline1.recorded_fingerprint
    );

    // Actually dispatching the merge now (as the run driver would on
    // `Dispatch`) picks up the mutated value, matching the full-refresh
    // oracle — the gate's `NoOp` above did not corrupt or stale-cache
    // anything a subsequent genuine dispatch relies on.
    let contribution = ContributionVerdict::Monotone;
    let bound = BoundResult::Bounded {
        source_partition_col: "d".to_string(),
        before: Seconds::ZERO,
        after: Seconds::hours(24),
    };
    let dimension_batch_sql = "SELECT e.d, e.user_id, e.val, u.tier \
         FROM main.events_enriched e JOIN main.sources_users u ON e.user_id = u.user_id";
    execute_column_scoped_merge(
        &backend,
        "main",
        "events_enriched",
        &["user_id".to_string()],
        &contribution,
        &bound,
        "d",
        "2024-01-01 12:00:00",
        dimension_batch_sql,
        &[],
        &unconditional(),
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("column-scoped merge must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let maintained_tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained tier for user 1");
    assert_eq!(maintained_tier_1, "gold");
}

/// The `PartitionLocal::Yes` corner's physical mechanism, end-to-end against
/// a real DuckDB backend: `decide_column_merge_dispatch` selects
/// `ColumnMergeDispatch::Clamped`, `widen_horizon_for_batch` derives the
/// horizon, and `execute_column_scoped_merge`/`dimension_horizon_merge`
/// (F15) actually clamp the MERGE to `[conv_ts − H, conv_ts]` on the
/// target's own partition axis — distinct from
/// `execute_column_scoped_merge_full`'s unconditional full merge above: a
/// mutated dimension row OUTSIDE the horizon must be left untouched this
/// run (the horizon settled-delay/tail-rewrite mechanism that would later
/// catch it up is unbuilt — `model_transforms.md`'s transform catalogue).
#[tokio::test]
async fn yes_corner_clamps_the_merge_to_the_horizon_and_leaves_the_rest_untouched() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (d DATE, user_id BIGINT, val DOUBLE, status VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             (DATE '2024-01-01', 1, 10.0, 'active'), \
             (DATE '2024-01-03', 2, 20.0, 'active')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_user_status (user_id BIGINT, status VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_user_status VALUES (1, 'suspended'), (2, 'suspended')",
        )
        .await
        .expect("seed dim table (both users mutated)");

    let cell = PlanCell {
        group: "{status}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "user_status".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![ScanClamp {
            source: "user_status".to_string(),
            column: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
            write_footprint: None,
        }],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    };

    let dispatch = decide_column_merge_dispatch(
        &cell,
        "user_status",
        /* table_exists */ true,
        /* model_declares_unique_key */ true,
        &ContributionVerdict::Monotone,
    )
    .expect("a PartitionLocal::Yes cell with a matching scan must dispatch Clamped");
    let ColumnMergeDispatch::Clamped(scan) = dispatch else {
        panic!("expected ColumnMergeDispatch::Clamped, got {dispatch:?}");
    };

    // A 1-day batch — the widened horizon must equal the derived 24h margin
    // (neither narrows the other here).
    let bound = widen_horizon_for_batch(&scan, Seconds::days(1));
    assert_eq!(
        bound,
        BoundResult::Bounded {
            source_partition_col: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        }
    );

    let dimension_batch_sql = "SELECT e.d, e.user_id, e.val, s.status \
         FROM main.events_enriched e JOIN main.sources_user_status s ON e.user_id = s.user_id";

    execute_column_scoped_merge(
        &backend,
        "main",
        "events_enriched",
        &["user_id".to_string()],
        &ContributionVerdict::Monotone,
        &bound,
        "d",
        "2024-01-01 00:00:00",
        dimension_batch_sql,
        &[],
        &unconditional(),
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("horizon-clamped column-scoped merge must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");

    let within_horizon: String = conn
        .query_row(
            "SELECT status FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained status for user 1");
    assert_eq!(
        within_horizon, "suspended",
        "d=2024-01-01 is within [conv_ts - 24h, conv_ts] — the mutation must be picked up"
    );

    let outside_horizon: String = conn
        .query_row(
            "SELECT status FROM main.events_enriched WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read status for user 2");
    assert_eq!(
        outside_horizon, "active",
        "d=2024-01-03 falls outside [conv_ts - 24h, conv_ts] — the horizon clamp must leave it \
         untouched this run, unlike execute_column_scoped_merge_full's unconditional full merge"
    );
}

/// Phase C4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed column-scoped MERGE (T1), real-DuckDB proof: an
/// unchanged-input re-run writes **zero** rows. `resolve_write_suppression`
/// is fed a proven `Key` row identity (P2) and an all-`Comparable` P3
/// verdict for the `{tier}` group — the admission this phase adds — so it
/// resolves `WriteSuppression::Suppressed`, and `execute_column_scoped_merge_
/// full` dispatches through `emit_column_scoped_merge_suppressed` instead of
/// the plain unconditional emitter used by the tests above.
///
/// The "zero rows written" proof reads DuckDB's own `Count` column off the
/// `MERGE`'s query result (`execute_sql`'s returned `RecordBatch`, captured
/// here directly rather than through `execute_column_scoped_merge_full`'s
/// `ExecutionResult::row_count`, which only reports the target's total row
/// count, not the number of rows the statement itself touched) — the same
/// affected-row semantics DuckDB's own `MERGE`/`UPDATE`/`INSERT` report.
#[tokio::test]
async fn suppressed_merge_writes_zero_rows_on_unchanged_rerun() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    // Dimension starts out identical to the target — the first run below is
    // itself an unchanged-input run.
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed dim table");

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";

    // The admission this phase adds: a proven key over a fully comparable
    // group resolves `Suppressed`, naming exactly the `{tier}` group.
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );
    assert_eq!(
        suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["tier".to_string()]
        },
        "a proven key over a fully comparable group must admit the conditional variant"
    );

    // Run 1: dimension unchanged relative to the target — the suppressed
    // MERGE must match every row but write none of them.
    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("suppressed column-scoped merge must succeed");

    let affected = merge_affected_row_count(
        &backend,
        "main.dim_users",
        "main.sources_users",
        &["user_id"],
        &["tier"],
    )
    .await;
    assert_eq!(
        affected, 0,
        "an unchanged-input re-run of the suppressed MERGE must write zero rows"
    );

    // Mutate the dimension for user 1 — now a real change exists.
    backend
        .execute_sql("UPDATE main.sources_users SET tier = 'gold' WHERE user_id = 1")
        .await
        .expect("mutate dimension");

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("suppressed column-scoped merge must succeed after mutation");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.dim_users WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained tier for user 1");
    assert_eq!(
        tier_1, "gold",
        "the suppressed MERGE must still pick up a genuine change"
    );
    let tier_2: String = conn
        .query_row(
            "SELECT tier FROM main.dim_users WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read unaffected tier for user 2");
    assert_eq!(tier_2, "silver", "an unmutated row must be left untouched");

    // Full-refresh oracle: the maintained state must equal a fresh join,
    // exactly like the unconditional-variant tests above.
    let oracle_tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.sources_users WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("oracle read");
    assert_eq!(tier_1, oracle_tier_1);

    // Run 3: dimension unchanged again (relative to the now-mutated state)
    // — zero rows written a second time.
    let affected_again = merge_affected_row_count(
        &backend,
        "main.dim_users",
        "main.sources_users",
        &["user_id"],
        &["tier"],
    )
    .await;
    assert_eq!(
        affected_again, 0,
        "a second unchanged-input re-run must also write zero rows"
    );
}

/// Issue the exact `emit_column_scoped_merge_suppressed`-shaped statement
/// directly and read DuckDB's own affected-row `Count` off the query
/// result — the same probe this test file's e2e proof reads its "zero rows
/// written" assertion from, kept separate from `execute_column_scoped_merge_
/// full` (whose `ExecutionResult::row_count` reports the target's total row
/// count, not the number of rows the statement itself touched).
async fn merge_affected_row_count(
    backend: &DuckDbBackend,
    target: &str,
    source: &str,
    key: &[&str],
    compared: &[&str],
) -> i64 {
    use smelt_logical::maintenance::emit::{
        emit_column_scoped_merge_suppressed, MaintenanceDialect,
    };

    let key_owned: Vec<String> = key.iter().map(|s| s.to_string()).collect();
    let compared_owned: Vec<String> = compared.iter().map(|s| s.to_string()).collect();
    let group = emit_column_scoped_merge_suppressed(
        target,
        &key_owned,
        &format!("SELECT * FROM {source}"),
        &compared_owned,
        &[],
        MaintenanceDialect::DuckDb,
    );
    let batches = backend
        .execute_sql(&group.statements[0].sql)
        .await
        .expect("probe merge must succeed");
    let batch = batches.first().expect("MERGE returns one Count row");
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("Count column is Int64");
    counts.value(0)
}

/// The staged-candidate `DELETE`+`INSERT` analogue of [`merge_affected_row_count`]
/// above (`docs/plans/20260808-membership-sensitivity.md` Phase 2): builds
/// the SAME [`smelt_logical::maintenance::emit::emit_staged_candidate_conditional`]
/// group the real dispatch (`maintenance_driver::
/// execute_staged_membership_recompute`) already ran, executes each
/// statement directly, and reads DuckDB's own affected-row `Count` off the
/// `DELETE`'s and `INSERT`'s own query results — proving an unchanged
/// redelivery's change-suppressed matched arm and NOT-EXISTS insert arm
/// both touch zero rows. `staged_relation` must be a name not already in
/// use (this helper does not check for a colliding relation; the caller's
/// own group has already run to completion by the time this probe fires).
pub(super) async fn staged_candidate_affected_row_counts(
    backend: &DuckDbBackend,
    target: &str,
    staged_relation: &str,
    key: &[&str],
    candidate_select: &str,
    compared: &[&str],
) -> (i64, i64) {
    use smelt_logical::maintenance::emit::{emit_staged_candidate_conditional, MaintenanceDialect};

    let key_owned: Vec<String> = key.iter().map(|s| s.to_string()).collect();
    let compared_owned: Vec<String> = compared.iter().map(|s| s.to_string()).collect();
    let group = emit_staged_candidate_conditional(
        target,
        staged_relation,
        &key_owned,
        candidate_select,
        &compared_owned,
        MaintenanceDialect::DuckDb,
    );
    // Statement order per `emit_staged_candidate_conditional`: [CREATE
    // staged, INSERT candidates into staged, DELETE changed, INSERT new,
    // DROP staged].
    backend
        .execute_sql(&group.statements[0].sql)
        .await
        .expect("probe: create staged relation");
    backend
        .execute_sql(&group.statements[1].sql)
        .await
        .expect("probe: insert candidates into staged relation");
    let deleted = {
        let batches = backend
            .execute_sql(&group.statements[2].sql)
            .await
            .expect("probe: staged-candidate DELETE must succeed");
        let batch = batches.first().expect("DELETE returns one Count row");
        let counts = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("Count column is Int64");
        counts.value(0)
    };
    let inserted = {
        let batches = backend
            .execute_sql(&group.statements[3].sql)
            .await
            .expect("probe: staged-candidate INSERT must succeed");
        let batch = batches.first().expect("INSERT returns one Count row");
        let counts = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("Count column is Int64");
        counts.value(0)
    };
    backend
        .execute_sql(&group.statements[4].sql)
        .await
        .expect("probe: drop staged relation");
    (deleted, inserted)
}

/// Phase G1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the conditional-variant dimension enters `choice.rs`'s override ladder:
/// `resolve_write_variant` folds the first-build/definition-change-backfill
/// posture into an already-proven `WriteSuppression::Suppressed` verdict.
/// This is the interchangeability claim the plan asks for, exercised
/// end-to-end against a real DuckDB backend: a steady-state trigger
/// (`Trigger::UpstreamMutation`, no ledger catch-up) *prefers* the
/// change-suppressed matched arm, while a first-build/backfill trigger
/// (`Trigger::Backfill`, or a definition-change cell's own
/// `ledger_catch_up: true`) is *admitted but not preferred* and resolves the
/// unconditional matched arm instead — bit-identical final state either way
/// (`incremental_models.md` §"Interchangeability and choice"), never a
/// difference in which `S` is reflected.
#[tokio::test]
async fn first_build_posture_and_steady_state_preference_resolve_bit_identical_state() {
    use smelt_logical::maintenance::choice::{resolve_write_variant, VariantReason};

    let tmp = tempfile::TempDir::new().expect("tempdir");

    // Two separate real DuckDB databases, seeded identically: a target that
    // already diverges from its dimension source for user 1 (tier
    // 'bronze' → 'gold'), unchanged for user 2.
    async fn seed(path: &Path) -> DuckDbBackend {
        let backend = DuckDbBackend::new(path, "main").await.expect("open duckdb");
        backend
            .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create target table");
        backend
            .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
            .await
            .expect("seed target table");
        backend
            .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create dim table");
        backend
            .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
            .await
            .expect("seed dim table");
        backend
    }

    let steady_db_path = tmp.path().join("steady.duckdb");
    let backfill_db_path = tmp.path().join("backfill.duckdb");
    let steady_backend = seed(&steady_db_path).await;
    let backfill_backend = seed(&backfill_db_path).await;

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let proven_suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );
    assert_eq!(
        proven_suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["tier".to_string()]
        },
        "precondition: the compare must actually be admitted for this test to prove anything"
    );

    // Steady-state trigger: no prior-state gap — the ladder PREFERS
    // suppression.
    let steady_trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (steady_variant, steady_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(steady_reason, VariantReason::SteadyStatePreference);
    assert!(matches!(
        steady_variant,
        WriteSuppression::Suppressed { .. }
    ));

    // First-build/backfill trigger: admitted, but NOT preferred — resolves
    // unconditional by default even though the same proof holds.
    let (backfill_variant, backfill_reason) = resolve_write_variant(
        &proven_suppression,
        &Trigger::Backfill,
        false,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(backfill_reason, VariantReason::FirstBuildPosture);
    assert!(matches!(
        backfill_variant,
        WriteSuppression::Unconditional { .. }
    ));

    // A definition-change backfill cell (`ledger_catch_up: true`) resolves
    // the same way even on an otherwise-steady-state trigger.
    let (catch_up_variant, catch_up_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        true,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(catch_up_reason, VariantReason::FirstBuildPosture);
    assert!(matches!(
        catch_up_variant,
        WriteSuppression::Unconditional { .. }
    ));

    // Execute both resolved variants against their own real, identically
    // seeded target — one via the suppressed matched arm the steady-state
    // trigger prefers, the other via the unconditional matched arm the
    // first-build/backfill posture defaults to.
    execute_column_scoped_merge_full(
        &steady_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &steady_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("steady-state suppressed merge must succeed");

    execute_column_scoped_merge_full(
        &backfill_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &backfill_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("first-build unconditional merge must succeed");

    // Bit-identical final state either way — choice may change which
    // matched-arm shape ran, never observable bits at a fixed processed-
    // input set.
    let steady_conn = duckdb::Connection::open(&steady_db_path).expect("reconnect steady");
    let backfill_conn = duckdb::Connection::open(&backfill_db_path).expect("reconnect backfill");
    for user_id in [1_i64, 2_i64] {
        let steady_tier: String = steady_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read steady tier");
        let backfill_tier: String = backfill_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read backfill tier");
        assert_eq!(
            steady_tier, backfill_tier,
            "user {user_id}: the suppressed (steady-state-preferred) and unconditional \
             (first-build-posture) matched arms must produce bit-identical state"
        );
    }
}

/// Phase G1's pin dimension: a `technique: unconditional` pin forces the
/// plain matched arm on a steady-state trigger that would otherwise prefer
/// suppression, and a `technique: suppress` pin forces the change-suppressed
/// matched arm on for a first-build trigger that would otherwise default to
/// unconditional — exercised end-to-end against a real DuckDB backend,
/// asserting bit-identical final state against the natural (unpinned)
/// resolution either way (`incremental_models.md` §"Interchangeability and
/// choice": a pin may change which matched-arm shape ran, never the bits at
/// a fixed processed-input set).
#[tokio::test]
async fn technique_pin_forces_the_variant_and_still_produces_bit_identical_state() {
    use smelt_core::config::CellTechnique;
    use smelt_logical::maintenance::choice::{
        resolve_write_variant, EffectiveOverride, VariantReason,
    };

    let tmp = tempfile::TempDir::new().expect("tempdir");

    async fn seed(path: &Path) -> DuckDbBackend {
        let backend = DuckDbBackend::new(path, "main").await.expect("open duckdb");
        backend
            .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create target table");
        backend
            .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
            .await
            .expect("seed target table");
        backend
            .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create dim table");
        backend
            .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
            .await
            .expect("seed dim table");
        backend
    }

    let natural_db_path = tmp.path().join("natural.duckdb");
    let pinned_db_path = tmp.path().join("pinned.duckdb");
    let natural_backend = seed(&natural_db_path).await;
    let pinned_backend = seed(&pinned_db_path).await;

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let proven_suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );

    // A steady-state trigger naturally prefers suppression — pin
    // `technique: unconditional` to force the plain matched arm instead.
    let steady_trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (natural_variant, natural_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(natural_reason, VariantReason::SteadyStatePreference);
    assert!(matches!(
        natural_variant,
        WriteSuppression::Suppressed { .. }
    ));

    let unconditional_pin = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Unconditional),
    };
    let (pinned_variant, pinned_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &unconditional_pin,
    )
    .expect("`technique: unconditional` is always admissible — never refuses");
    assert_eq!(pinned_reason, VariantReason::Overridden);
    assert!(matches!(
        pinned_variant,
        WriteSuppression::Unconditional { .. }
    ));

    execute_column_scoped_merge_full(
        &natural_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &natural_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("natural suppressed merge must succeed");

    execute_column_scoped_merge_full(
        &pinned_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &pinned_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("pinned unconditional merge must succeed");

    let natural_conn = duckdb::Connection::open(&natural_db_path).expect("reconnect natural");
    let pinned_conn = duckdb::Connection::open(&pinned_db_path).expect("reconnect pinned");
    for user_id in [1_i64, 2_i64] {
        let natural_tier: String = natural_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read natural tier");
        let pinned_tier: String = pinned_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read pinned tier");
        assert_eq!(
            natural_tier, pinned_tier,
            "user {user_id}: the natural (suppressed) and pinned (unconditional) matched \
             arms must produce bit-identical state — a pin changes which shape ran, never \
             the bits at a fixed processed-input set"
        );
    }
}
