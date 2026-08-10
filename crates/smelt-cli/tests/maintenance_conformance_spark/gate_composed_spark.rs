//! Spark twin of `maintenance_conformance/gate.rs`'s composed keyed pool
//! (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5).
//!
//! Only `ComposedRoute::RecurrenceBounded` (route 3, a `MAX` cross-partition
//! combiner) drives an equivalence check on Spark: it grades
//! `Grade::Idempotent`, so its direct-driver execution through
//! `run_windowed_keyed_maintenance` (generic over `&dyn Backend` — the same
//! DuckDB-only workaround `gate.rs` documents ports unchanged) needs no
//! ledger. `ComposedRoute::KeyEmbedded` (route 1, `execute_project`) and
//! `ComposedRoute::KeyDetermined` (route 2, direct-driver) both use a `SUM`
//! cross-partition combiner (`total`) and so BOTH grade `Grade::Additive` —
//! confirmed live against Spark (route 1's `execute_project` drive failed
//! with the driver's own fail-loud `BackendError::unsupported`, the same
//! never-fold-twice reconciliation-ledger gap already excluding
//! `KeyedCombiner::Additive` from `gate_keyed_spark.rs`; see this plan's
//! "Deferred during implementation" entry, extended to cover routes 1 and 2
//! here too). Admission (classification only, never execution) is unaffected
//! by that gap, so the admission-rate twin below still samples all three
//! routes.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_maintenance_testkit::link_c_harness::LinkCProject;
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_composed_route, arb_composed_route3_schedule, ComposedKeyedRecipe, ComposedRoute,
    ComposedRoute3Schedule, ConformanceTarget, SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::schedule_gen::GenRow;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::check_runner::batches_to_rows;
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

use crate::gate_spark::spark_connect_url;

/// Default deterministic case count for
/// `composed_keyed_pool_upholds_equivalence_on_spark` (route 3 only) —
/// `SMELT_CONFORMANCE_SPARK_COMPOSED_CASES` env override.
const DEFAULT_COMPOSED_CASES: usize = 4;

fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_COMPOSED_CASES)
}

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "maintenance-conformance-gate-spark",
        model_name: "maintenance-conformance-gate-spark",
        reporter: &NO_OP_REPORTER,
    }
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — copied from `maintenance_conformance/gate.rs::classify_composed_full`
/// (target-agnostic: reads the staged project's own files off disk, never
/// touches a backend; see `gate_keyed_spark.rs`'s doc comment for why this is
/// a copy rather than a shared import).
fn classify_composed_full(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged composed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Copied from `gate.rs::assert_composed_admitted_with_expected_route` —
/// pure over the derived plan, no backend dependency.
fn assert_composed_admitted_with_expected_route(
    recipe: &ComposedKeyedRecipe,
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> anyhow::Result<()> {
    if plan.refusals.iter().any(|r| {
        matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )
    }) {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) was refused by the locality gate: {:?}",
            recipe.model_name,
            recipe.route,
            plan.refusals
        );
    }
    let Some(key_locality) = plan.key_locality.as_ref() else {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) admitted a plan with no key_locality",
            recipe.model_name,
            recipe.route
        );
    };
    match (recipe.route, &key_locality.slice) {
        (ComposedRoute::KeyEmbedded, LocalitySlice::Window { .. }) => Ok(()),
        (ComposedRoute::KeyDetermined, LocalitySlice::DeltaValues { .. }) => Ok(()),
        (ComposedRoute::RecurrenceBounded, LocalitySlice::RecurrenceBounded { .. }) => Ok(()),
        (route, slice) => {
            anyhow::bail!(
                "composed recipe {:?}: route {:?} admitted an unexpected slice shape: {:?}",
                recipe.model_name,
                route,
                slice
            )
        }
    }
}

fn stage_composed_spark(
    recipe: &ComposedKeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_composed_for_target(
        recipe,
        &project_dir,
        &db_path,
        ConformanceTarget::SparkDelta,
    )
}

// ---- Route 3 (recurrence-bounded): direct-driver execution on Spark ----

fn composed_driving_timeseries() -> smelt_core::config::TimeseriesConfig {
    smelt_core::config::TimeseriesConfig {
        event_time_column: "d".to_string(),
        partition_column: "d".to_string(),
        granularity: smelt_core::config::Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn composed_route3_classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

fn composed_route3_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    }
}

fn composed_route3_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["last_seen".to_string()],
    }
}

fn composed_delta_values_sql(rows: &[GenRow]) -> String {
    let values: Vec<String> = rows
        .iter()
        .map(|r| format!("({}, DATE '{}', {})", r.id, r.d.format("%Y-%m-%d"), r.val))
        .collect();
    format!("(VALUES {}) AS t(id, d, val)", values.join(", "))
}

fn composed_route3_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {} GROUP BY id",
        composed_delta_values_sql(rows)
    )
}

fn composed_route3_oracle_sql_spark(source_name: &str) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {SPARK_CONFORMANCE_SCHEMA}.sources_{source_name} \
         GROUP BY id"
    )
}

async fn assert_composed_route3_equivalence_spark(
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!(
        "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
        recipe.model_name
    );
    let oracle_sql = composed_route3_oracle_sql_spark(&recipe.source.name);
    if !multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await? {
        anyhow::bail!(
            "Spark composed route-3 equivalence violated for {:?}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

async fn assert_composed_route3_per_slice_spark(
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(last_seen AS STRING) AS v FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{} WHERE last_seen = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE last_seen = DATE '{v}'",
            composed_route3_oracle_sql_spark(&recipe.source.name)
        );
        if !multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await? {
            anyhow::bail!(
                "Spark composed route-3 per-slice equivalence violated for {:?} at last_seen \
                 {v}: maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                recipe.model_name
            );
        }
    }
    Ok(())
}

async fn insert_composed_rows_via_backend_spark(
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
    rows: &[GenRow],
) -> anyhow::Result<()> {
    for row in rows {
        backend
            .execute_sql(&format!(
                "INSERT INTO {SPARK_CONFORMANCE_SCHEMA}.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                row.d.format("%Y-%m-%d"),
                row.id,
                row.val,
            ))
            .await
            .map_err(|e| anyhow::anyhow!("insert composed route-3 oracle row on Spark: {e}"))?;
    }
    Ok(())
}

async fn drive_composed_route3_and_assert_spark(
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
    schedule: &ComposedRoute3Schedule,
) -> anyhow::Result<()> {
    let classification = composed_route3_classification(recipe);
    let slice = composed_route3_slice();

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend_spark(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route3_delta_sql(&rows))
        };
        let run_date_str = window.run_date.format("%Y-%m-%d").to_string();
        let next_day_str = (window.run_date + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let steps = driving_steps(
            &run_date_str,
            &next_day_str,
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            SPARK_CONFORMANCE_SCHEMA,
            &recipe.model_name,
            &steps,
            &classification,
            Some(&slice),
            &composed_route3_suppression(),
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Spark composed route-3 window {i} (in-bound redelivery) unexpectedly refused: {e}"
            )
        })?;

        assert_composed_route3_equivalence_spark(backend, recipe).await?;
        assert_composed_route3_per_slice_spark(backend, recipe).await?;
    }
    Ok(())
}

/// `composed_keyed_pool_upholds_equivalence_on_spark` (plan Phase 5 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::composed_keyed_pool_upholds_equivalence`
/// — route 3 (direct-driver) only; see this module's doc comment for why
/// routes 1/2 are excluded.
#[test]
fn composed_keyed_pool_upholds_equivalence_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping composed_keyed_pool_upholds_equivalence_on_spark"
        );
        return;
    };

    let n = composed_case_count();
    let mut runner = TestRunner::deterministic();
    let route3_schedule_strat = arb_composed_route3_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let recipe = ComposedKeyedRecipe::new(ComposedRoute::RecurrenceBounded);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_composed_spark(&recipe, &tmp).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage on Spark: {e}")
        });

        let (plan, diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed on Spark: {e}")
        });
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no plan derived for {recipe:?}: diagnostics={diags:#?}")
        });
        assert_composed_admitted_with_expected_route(&recipe, &plan).unwrap_or_else(|e| {
            panic!("case {i}: {e}");
        });

        let schedule = route3_schedule_strat
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let backend = rt
            .block_on(project.backend_for_target(ConformanceTarget::SparkDelta))
            .expect("open spark backend");
        rt.block_on(drive_composed_route3_and_assert_spark(
            backend.as_ref(),
            &recipe,
            &schedule,
        ))
        .unwrap_or_else(|e| {
            panic!(
                "case {i}: Spark composed route-3 recipe {recipe:?} schedule {schedule:?} \
                 failed: {e}"
            )
        });
    }
}

/// `composed_keyed_admission_rate_stays_above_floor_on_spark` (plan Phase 5
/// TDD list): the Spark twin of
/// `maintenance_conformance::gate::composed_keyed_admission_rate_stays_above_floor`.
/// Samples ALL THREE routes (including `KeyEmbedded`/`KeyDetermined`) —
/// admission is pure classification, never execution, so the
/// `Grade::Additive` ledger gap that excludes those routes from the
/// equivalence drive above does not apply here.
#[test]
fn composed_keyed_admission_rate_stays_above_floor_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             composed_keyed_admission_rate_stays_above_floor_on_spark"
        );
        return;
    };

    let n: usize = std::env::var("SMELT_CONFORMANCE_SPARK_COMPOSED_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();

    let mut admitted = 0;
    for i in 0..n {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_composed_spark(&recipe, &tmp).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage on Spark: {e}")
        });
        let (plan, _diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed on Spark: {e}")
        });
        let admitted_here = plan
            .map(|p| assert_composed_admitted_with_expected_route(&recipe, &p).is_ok())
            .unwrap_or(false);
        if admitted_here {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / n as f64;
    assert!(
        rate >= 0.90,
        "Spark composed-pool admission rate {rate:.2} over N={n} fell below the 90% floor \
         ({admitted}/{n} admitted)"
    );
}
