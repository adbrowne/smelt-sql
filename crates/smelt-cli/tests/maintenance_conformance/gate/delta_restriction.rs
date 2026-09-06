//! Delta-restriction (T3) vs widened-scan equivalence at a fixed processed-input set, and the empty-delta no-op cascade end to end.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::support::no_retry_policy;
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::Trigger;
use smelt_maintenance_testkit::recipe::{
    arb_enrichment_edge_recipe, arb_enrichment_edge_schedule, EnrichmentJoinKind,
};
use smelt_runtime::maintenance_driver::RestrictionDeltaSource;

// =============================================================================
// Phase E4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
// — delta-restriction (T3) vs widened-scan equivalence at a fixed
// processed-input set `S`, plus the empty-delta no-op cascade end to end.
// Both legs exercise REAL production entry points directly
// (`append_model_edge_cells`, `execute_delete_insert_with_delta_
// restriction`, `execute_column_scoped_merge_full`,
// `plan_since_upstream_with_observed_deltas`) rather than a hand-rolled
// reimplementation — the same "direct fact injection" discipline
// `crates/smelt-runtime/tests/delta_restricted_recompute.rs` (E3) and
// `crates/smelt-runtime/tests/since_upstream_propagation.rs`'s D3 tests
// already use, generalized to a generated sample.
// =============================================================================

/// Total baseline keys per generated case for `EnrichmentEdgeRecipe` —
/// `arb_enrichment_edge_schedule`'s own `1..total` non-empty-proper-subset
/// contract needs `total >= 2`.
pub(crate) const ENRICHMENT_TOTAL_KEYS: usize = 6;

pub(crate) const ENRICHMENT_DEFAULT_CASES: usize = 12;

pub(crate) fn enrichment_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ENRICHMENT_DEFAULT_CASES)
}

/// Derive the REAL P1 skeleton-source-closure verdict for a
/// `EnrichmentJoinKind`'s own model-edge scope, through the SAME production
/// entry point `crates/smelt-runtime/tests/
/// web_analytics_session_delta_restriction.rs` exercises
/// (`append_model_edge_cells`), never a hand-typed classification.
pub(crate) fn enrichment_edge_closed(join_kind: EnrichmentJoinKind) -> bool {
    let recipe = smelt_maintenance_testkit::recipe::EnrichmentEdgeRecipe::new(join_kind);
    let mut plan = smelt_logical::maintenance::MaintenancePlan::default();
    smelt_logical::maintenance::derive::append_model_edge_cells(
        &mut plan,
        &recipe.model_body(),
        Some("event_date"),
        &recipe.model_edges(),
        &[],
        &[],
        &Default::default(),
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: recipe.driving_source().to_string(),
        })
        .unwrap_or_else(|| panic!("{join_kind:?} produced no model-edge creation cell"));
    cell.skeleton_source_closure
        .as_ref()
        .is_some_and(|c| c.is_closed())
}

/// Seed `main.<table>` with [`ENRICHMENT_TOTAL_KEYS`] baseline rows shaped
/// like `web_analytics_session_delta_restriction.rs`'s own
/// `events_enriched` fixture; a key in `schedule.touched_indices` gets its
/// `event_utm_campaign` value suffixed by `touched_suffix` (empty for a
/// plain baseline, `"-NEW"` for the recompute source that actually changed).
pub(crate) async fn seed_enrichment_case(
    backend: &DuckDbBackend,
    table: &str,
    schedule: &smelt_maintenance_testkit::recipe::EnrichmentEdgeSchedule,
    touched_suffix: &str,
) {
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.{table} (event_id VARCHAR, device_id VARCHAR, event_date DATE, \
             event_utm_campaign VARCHAR, session_id VARCHAR, session_utm_campaign VARCHAR)"
        ))
        .await
        .unwrap();
    let rows: Vec<String> = (0..ENRICHMENT_TOTAL_KEYS)
        .map(|k| {
            let campaign = if schedule.touched_indices.contains(&k) {
                format!("campaign-{k}{touched_suffix}")
            } else {
                format!("campaign-{k}")
            };
            format!("('ev-{k}', 'dev-{k}', '2026-07-01', '{campaign}', 'sess-{k}', 'campaign-{k}')")
        })
        .collect();
    backend
        .execute_sql(&format!(
            "INSERT INTO main.{table} VALUES {}",
            rows.join(", ")
        ))
        .await
        .unwrap();
}

pub(crate) async fn read_enrichment_rows(
    backend: &DuckDbBackend,
    table: &str,
) -> Vec<(String, String)> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT event_id, event_utm_campaign FROM main.{table} ORDER BY event_id"
        ))
        .await
        .unwrap();
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let campaigns = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push((ids.value(i).to_string(), campaigns.value(i).to_string()));
        }
    }
    out
}

pub(crate) async fn record_enrichment_delta(
    backend: &DuckDbBackend,
    upstream: &str,
    start: &str,
    end: &str,
    changed_keys: &[String],
) {
    let ensure = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ensure).await.unwrap();
    let changed_keys_query = if changed_keys.is_empty() {
        "SELECT NULL AS delta_key, NULL AS delta_partition WHERE FALSE".to_string()
    } else {
        let keys_list = changed_keys
            .iter()
            .map(|k| format!("('{k}', NULL)"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT * FROM (VALUES {keys_list}) AS t(delta_key, delta_partition)")
    };
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        upstream,
        start,
        end,
        &changed_keys_query,
    );
    backend.execute_sql(&upsert).await.unwrap();
}

/// `delta_restricted_equals_widened_scan_at_fixed_s` (Phase E4 TDD list):
/// over the closure-admitted subset of a generated `EnrichmentEdgeRecipe`
/// sample, force the SAME schedule through both `execute_delete_insert_
/// with_delta_restriction` dispatch outcomes — `Closed` (restricted) and a
/// forced `Open` (widened) — against two independently-seeded-identical
/// baselines, and assert bit-identical end state. This holds precisely
/// because every key OUTSIDE the schedule's touched set carries a
/// recompute-source value already identical to what is stored — recomputing
/// it (widened) reproduces exactly what leaving it alone (restricted)
/// would.
#[tokio::test]
async fn delta_restricted_equals_widened_scan_at_fixed_s() {
    let n = enrichment_case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_enrichment_edge_recipe();
    let schedule_strat = arb_enrichment_edge_schedule(ENRICHMENT_TOTAL_KEYS);

    let mut admitted = 0;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();

        let closed = enrichment_edge_closed(recipe.join_kind);
        assert_eq!(
            closed,
            recipe.expects_closed(),
            "case {i}: {recipe:?} P1 verdict ({closed}) did not match the recipe's own \
             closure-admissibility expectation"
        );
        if !closed {
            continue;
        }
        admitted += 1;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");

        let untouched_baseline = smelt_maintenance_testkit::recipe::EnrichmentEdgeSchedule {
            touched_indices: vec![],
        };
        seed_enrichment_case(&backend, "restricted_target", &untouched_baseline, "").await;
        seed_enrichment_case(&backend, "widened_target", &untouched_baseline, "").await;
        seed_enrichment_case(&backend, "enrichment_recompute", &schedule, "-NEW").await;

        let changed_keys: Vec<String> = schedule
            .touched_indices
            .iter()
            .map(|k| format!("ev-{k}"))
            .collect();
        record_enrichment_delta(
            &backend,
            recipe.driving_source(),
            "2026-07-01",
            "2026-07-02",
            &changed_keys,
        )
        .await;

        let region = smelt_logical::maintenance::emit::Region {
            start: "'2026-07-01'".to_string(),
            end: "'2026-07-02'".to_string(),
        };
        let body = "SELECT event_id, device_id, event_date, event_utm_campaign, session_id, \
                     session_utm_campaign FROM main.enrichment_recompute";

        let closed_verdict = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
            row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
        };
        smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
            &backend,
            "main",
            "restricted_target",
            "event_date",
            &region,
            body,
            body,
            Some("event_id"),
            Some(&closed_verdict),
            RestrictionDeltaSource::ModelEdge {
                upstream_model: recipe.driving_source(),
                window_start: "2026-07-01",
                window_end: "2026-07-02",
            },
            None,
            smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
            &[],
            &[],
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: restricted recompute failed: {e}"));

        let open_verdict = smelt_logical::maintenance::SkeletonSourceClosure::Open {
            reason: "forced widened-scan comparison".to_string(),
        };
        smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
            &backend,
            "main",
            "widened_target",
            "event_date",
            &region,
            body,
            body,
            Some("event_id"),
            Some(&open_verdict),
            RestrictionDeltaSource::ModelEdge {
                upstream_model: recipe.driving_source(),
                window_start: "2026-07-01",
                window_end: "2026-07-02",
            },
            None,
            smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
            &[],
            &[],
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: widened recompute failed: {e}"));

        let restricted_rows = read_enrichment_rows(&backend, "restricted_target").await;
        let widened_rows = read_enrichment_rows(&backend, "widened_target").await;
        assert_eq!(
            restricted_rows, widened_rows,
            "case {i}: {recipe:?} schedule {schedule:?} — delta-restricted and widened-scan \
             recomputes must be bit-identical at fixed S"
        );
    }

    assert!(
        admitted > 0,
        "N={n} deterministic sample admitted zero closure-Closed cases — generator/proof \
         regression"
    );
}

/// `delta_restriction_admission_rate_stays_above_floor` (Phase E4 TDD list):
/// exactly one of the three `EnrichmentJoinKind` variants (`LeftJoin`) is
/// closure-admissible by construction, so a uniform draw over N=30 should
/// land close to 33%; a 15% floor catches a generator or P1 regression
/// (`InnerJoin`/`MembershipPredicate` spuriously admitting, or `LeftJoin`
/// spuriously refusing) with wide margin against sampling noise, without
/// being flaky (`TestRunner::deterministic()` reproduces the SAME sequence
/// every run).
#[test]
fn delta_restriction_admission_rate_stays_above_floor() {
    const N: usize = 30;
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_enrichment_edge_recipe();

    let mut admitted = 0;
    for _ in 0..N {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        if enrichment_edge_closed(recipe.join_kind) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.15,
        "delta-restriction admission rate {rate:.2} over N={N} fell below the 15% floor \
         ({admitted}/{N} admitted) — a route-admission regression would silently hollow out the \
         standing gate"
    );
}

// =============================================================================
// `empty_delta_cascade_is_a_no_op` (Phase E4): the end-to-end payoff — a
// fully-suppressed conditional write over a composed (timeseries-
// partitioned) model-edge upstream records a REAL present-and-empty
// observed delta (T5, via the real `execute_column_scoped_merge_full`
// entry point, not a hand-typed record), which schedules ZERO downstream
// regions across a real fan-out cascade (`examples/timeseries`'s real
// `user_daily_spend -> {user_spend_rollup, user_spend_running_total}`
// graph, the same real fixture `crates/smelt-runtime/tests/
// since_upstream_propagation.rs`'s D3 tests exercise), and leaves the
// target byte-identical to a from-scratch full-refresh oracle.
// =============================================================================

pub(crate) fn key_suppression_for(compared: &[&str]) -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: compared.iter().map(|c| c.to_string()).collect(),
    }
}

pub(crate) async fn read_spend_rows(
    backend: &DuckDbBackend,
    table: &str,
) -> Vec<(i64, String, f64)> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT user_id, spend_date::VARCHAR, total_amount FROM main.{table} ORDER BY user_id"
        ))
        .await
        .unwrap();
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, Float64Array, Int64Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let dates = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let amounts = batch
            .column(2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push((ids.value(i), dates.value(i).to_string(), amounts.value(i)));
        }
    }
    out
}

#[tokio::test]
async fn empty_delta_cascade_is_a_no_op() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("db.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    // The upstream: `user_daily_spend`-styled (`examples/timeseries`'s own
    // model name and columns), already processed for 2026-07-01.
    backend
        .execute_sql(
            "CREATE TABLE main.user_daily_spend (user_id BIGINT, spend_date DATE, total_amount \
             DOUBLE)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.user_daily_spend VALUES (1, '2026-07-01', 10.0), \
             (2, '2026-07-01', 20.0), (3, '2026-07-01', 30.0)",
        )
        .await
        .unwrap();

    // A redelivery of the SAME window: byte-identical to what is stored —
    // an upstream run that changes nothing.
    backend
        .execute_sql(
            "CREATE TABLE main.user_daily_spend_recompute (user_id BIGINT, spend_date DATE, \
             total_amount DOUBLE)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.user_daily_spend_recompute VALUES (1, '2026-07-01', 10.0), \
             (2, '2026-07-01', 20.0), (3, '2026-07-01', 30.0)",
        )
        .await
        .unwrap();

    let suppression = key_suppression_for(&["total_amount"]);
    let dimension_batch_sql =
        "SELECT user_id, spend_date, total_amount FROM main.user_daily_spend_recompute";
    let window = smelt_backend::PartitionRange {
        column: "spend_date".to_string(),
        start: "2026-07-01".to_string(),
        end: "2026-07-02".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };

    // Leg (a): the write itself, executed for real — snapshot the target
    // before, run the real conditional write + record entry point, snapshot
    // after. Zero writes, not merely zero net diffs.
    let before = read_spend_rows(&backend, "user_daily_spend").await;
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "user_daily_spend",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &window,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge over an unchanged redelivery must succeed");
    let after = read_spend_rows(&backend, "user_daily_spend").await;
    assert_eq!(
        before, after,
        "an unchanged redelivery must write zero rows — the target's state must be \
         byte-identical before and after"
    );

    // The REAL recorded delta (T5) — read back through the same production
    // entry point `crates/smelt-runtime/tests/observed_delta.rs` exercises,
    // never hand-typed.
    let changed_keys = smelt_runtime::maintenance_driver::read_observed_delta_changed_keys(
        &backend,
        "main",
        "user_daily_spend",
        "2026-07-01",
        "2026-07-02",
    )
    .await
    .expect("read observed delta")
    .expect("a fully-suppressed run must record a present (not absent) delta");
    assert!(
        changed_keys.is_empty(),
        "a fully-suppressed run must record an EMPTY changed-key set: {changed_keys:?}"
    );

    // Leg (c): the full-refresh oracle — an independent, from-scratch
    // recompute over the SAME (unchanged) source data — still matches.
    backend
        .execute_sql(
            "CREATE TABLE main.oracle_daily_spend AS SELECT user_id, spend_date, total_amount \
             FROM main.user_daily_spend_recompute",
        )
        .await
        .unwrap();
    let mut sorted_after = after.clone();
    sorted_after.sort_by_key(|r| r.0);
    let mut sorted_oracle = read_spend_rows(&backend, "oracle_daily_spend").await;
    sorted_oracle.sort_by_key(|r| r.0);
    assert_eq!(
        sorted_after, sorted_oracle,
        "the no-op run's end state must still equal a from-scratch full-refresh oracle"
    );

    // Leg (b): the REAL propagation graph — `examples/timeseries`'s actual
    // `user_daily_spend -> {user_spend_rollup, user_spend_running_total}`
    // fan-out — feeding the delta this test JUST recorded for real must
    // schedule ZERO regions across the WHOLE cascade, not just its own
    // edge.
    let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &["models".to_string()]);

    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| {
            a == "user_daily_spend" || a == "user_spend_rollup" || a == "user_spend_running_total"
        })
        .collect();
    assert_eq!(
        order.len(),
        3,
        "expected all three real fixture models to be discovered: {order:?}"
    );

    let window_interval = smelt_logical::maintenance::propagate::PartitionInterval::new(
        smelt_logical::maintenance::propagate::day_start(
            smelt_logical::maintenance::propagate::day_ordinal(2026, 7, 1),
        ),
        smelt_logical::maintenance::propagate::day_start(
            smelt_logical::maintenance::propagate::day_ordinal(2026, 7, 2),
        ),
    );
    let deltas = vec![smelt_runtime::propagation::SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: window_interval,
    }];
    let mut observed = smelt_runtime::propagation::ObservedDeltaLookup::new();
    observed.insert(
        (
            "user_daily_spend".to_string(),
            "2026-07-01".to_string(),
            "2026-07-02".to_string(),
        ),
        smelt_state::ddl_duckdb::ObservedDelta {
            changed_keys,
            partitions: vec![],
        },
    );

    let plan = smelt_runtime::propagation::plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
        "2026-08-01",
    )
    .expect("a present-and-empty observed delta must not be a refusal");

    assert!(
        plan.runs.is_empty(),
        "a fully-suppressed upstream run must schedule ZERO downstream regions across the whole \
         cascade (both user_spend_rollup and user_spend_running_total): {:?}",
        plan.runs
    );
}
