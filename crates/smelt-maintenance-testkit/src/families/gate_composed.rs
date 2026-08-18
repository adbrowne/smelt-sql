//! The composed keyed pool family (`ComposedKeyedRecipe`) — the
//! target-parametrized twin of `maintenance_conformance/gate.rs`'s composed
//! leg.
//!
//! Only `ComposedRoute::RecurrenceBounded` (route 3, a `MAX` cross-partition
//! combiner) drives an equivalence check: it grades `Grade::Idempotent`, so
//! its direct-driver execution through `run_windowed_keyed_maintenance`
//! (generic over `&dyn Backend`) needs no ledger. `ComposedRoute::KeyEmbedded`
//! (route 1, `execute_project`) and `ComposedRoute::KeyDetermined` (route 2,
//! direct-driver) both use a `SUM` cross-partition combiner (`total`) and so
//! BOTH grade `Grade::Additive`, which fails loud with
//! `BackendError::unsupported` on any non-DuckDB dialect (MP12's ledger DDL
//! has no such dialect yet — the same gap that already excludes
//! `KeyedCombiner::Additive` from `gate_keyed`). Admission (classification
//! only, never execution) is unaffected by that gap, so the admission-rate
//! family below still samples all three routes.

use anyhow::Result;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::check_runner::batches_to_rows;
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

use crate::families::ConformanceBackend;
use crate::link_c_harness::LinkCProject;
use crate::oracle::multiset_equal_via_backend_with_diff;
use crate::recipe::{
    arb_composed_route, arb_composed_route3_schedule, ComposedKeyedRecipe, ComposedRoute,
    ComposedRoute3Schedule,
};
use crate::render;
use crate::schedule_gen::GenRow;

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "maintenance-conformance-gate-composed",
        model_name: "maintenance-conformance-gate-composed",
        reporter: &NO_OP_REPORTER,
    }
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — target-agnostic: reads the staged project's own files off
/// disk, never touches a backend.
pub fn classify_composed_full(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> Result<(
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

/// Pure over the derived plan, no backend dependency.
pub fn assert_composed_admitted_with_expected_route(
    recipe: &ComposedKeyedRecipe,
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> Result<()> {
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

pub fn stage_composed_for(
    recipe: &ComposedKeyedRecipe,
    tmp: &tempfile::TempDir,
    target: crate::recipe::ConformanceTarget,
) -> Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_composed_for_target(recipe, &project_dir, &db_path, target)
}

// ---- Route 3 (recurrence-bounded): direct-driver execution ----

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

/// The route-3 delta's inline row set, dialect-aware
/// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7) —
/// routed through `smelt_core::sql::row_set::build_row_set_table`, the
/// single dialect-aware owner, rather than hand-formatting a table-value
/// constructor directly: GoogleSQL has no table-value constructor in `FROM` position
/// (`400 Syntax error: Expected keyword JOIN but got ","`, measured live
/// against BigQuery). `dialect` comes from
/// [`ConformanceBackend::dialect`] — DuckDB/Spark keep today's `(VALUES
/// …) AS t(id, d, val)` shape byte-identical; BigQuery gets the portable
/// `(SELECT … UNION ALL SELECT …) AS t` rewrite instead.
fn composed_delta_values_sql(dialect: smelt_core::config::BackendType, rows: &[GenRow]) -> String {
    let rows_sql: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.to_string(),
                format!("DATE '{}'", r.d.format("%Y-%m-%d")),
                r.val.to_string(),
            ]
        })
        .collect();
    smelt_core::sql::row_set::build_row_set_table(dialect, "t", &["id", "d", "val"], &rows_sql)
}

fn composed_route3_delta_sql(dialect: smelt_core::config::BackendType, rows: &[GenRow]) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {} GROUP BY id",
        composed_delta_values_sql(dialect, rows)
    )
}

fn composed_route3_oracle_sql(schema: &str, source_name: &str) -> String {
    format!("SELECT id, MAX(d) AS last_seen FROM {schema}.sources_{source_name} GROUP BY id")
}

async fn assert_composed_route3_equivalence_for(
    b: &dyn ConformanceBackend,
    schema: &str,
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
) -> Result<()> {
    let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
    let oracle_sql = composed_route3_oracle_sql(schema, &recipe.source.name);
    if !multiset_equal_via_backend_with_diff(backend, &maintained_sql, &oracle_sql, |l, r| {
        b.multiset_diff_sql(l, r)
    })
    .await?
    {
        anyhow::bail!(
            "composed route-3 equivalence violated for {:?}: maintained ({maintained_sql:?}) \
             != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

async fn assert_composed_route3_per_slice_for(
    b: &dyn ConformanceBackend,
    schema: &str,
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
) -> Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(last_seen AS STRING) AS v FROM {schema}.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM {schema}.{} WHERE last_seen = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE last_seen = DATE '{v}'",
            composed_route3_oracle_sql(schema, &recipe.source.name)
        );
        if !multiset_equal_via_backend_with_diff(backend, &maintained_sql, &oracle_sql, |l, r| {
            b.multiset_diff_sql(l, r)
        })
        .await?
        {
            anyhow::bail!(
                "composed route-3 per-slice equivalence violated for {:?} at last_seen {v}: \
                 maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                recipe.model_name
            );
        }
    }
    Ok(())
}

async fn insert_composed_rows_via_backend_for(
    schema: &str,
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
    rows: &[GenRow],
) -> Result<()> {
    for row in rows {
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                row.d.format("%Y-%m-%d"),
                row.id,
                row.val,
            ))
            .await
            .map_err(|e| anyhow::anyhow!("insert composed route-3 oracle row: {e}"))?;
    }
    Ok(())
}

async fn drive_composed_route3_and_assert_for(
    b: &dyn ConformanceBackend,
    schema: &str,
    backend: &dyn Backend,
    recipe: &ComposedKeyedRecipe,
    schedule: &ComposedRoute3Schedule,
) -> Result<()> {
    let classification = composed_route3_classification(recipe);
    let slice = composed_route3_slice();
    let dialect = b.dialect();

    for (i, window) in schedule.0.iter().enumerate() {
        b.before_step().await;
        insert_composed_rows_via_backend_for(schema, backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route3_delta_sql(dialect, &rows))
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
        b.before_step().await;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            schema,
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
                "composed route-3 window {i} (in-bound redelivery) unexpectedly refused: {e}"
            )
        })?;

        assert_composed_route3_equivalence_for(b, schema, backend, recipe).await?;
        assert_composed_route3_per_slice_for(b, schema, backend, recipe).await?;
    }
    Ok(())
}

/// `composed_keyed_pool_upholds_equivalence` — the twin of
/// `maintenance_conformance::gate::composed_keyed_pool_upholds_equivalence`
/// — route 3 (direct-driver) only; see this module's doc comment for why
/// routes 1/2 are excluded.
pub async fn run_composed_keyed_pool_upholds_equivalence(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let route3_schedule_strat = arb_composed_route3_schedule();

    for i in 0..n {
        let recipe = ComposedKeyedRecipe::new(ComposedRoute::RecurrenceBounded);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_composed_for(&recipe, &tmp, b.target(i)).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });

        let (plan, diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
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
        let target = b.target(i);
        let schema = b.schema(i);
        let backend = project
            .backend_for_target(target)
            .await
            .expect("open backend");
        drive_composed_route3_and_assert_for(b, &schema, backend.as_ref(), &recipe, &schedule)
            .await
            .unwrap_or_else(|e| {
                panic!("case {i}: composed route-3 recipe {recipe:?} schedule {schedule:?} failed: {e}")
            });
    }
    Ok(())
}

/// `composed_keyed_admission_rate_stays_above_floor` — the twin of
/// `maintenance_conformance::gate::composed_keyed_admission_rate_stays_above_floor`.
/// Samples ALL THREE routes (including `KeyEmbedded`/`KeyDetermined`) —
/// admission is pure classification, never execution, so the
/// `Grade::Additive` ledger gap that excludes those routes from the
/// equivalence drive above does not apply here.
pub async fn run_composed_keyed_admission_rate_stays_above_floor(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();

    let mut admitted = 0;
    for i in 0..n {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_composed_for(&recipe, &tmp, b.target(i)).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });
        let (plan, _diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let admitted_here = plan
            .map(|p| assert_composed_admitted_with_expected_route(&recipe, &p).is_ok())
            .unwrap_or(false);
        if admitted_here {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / n as f64;
    anyhow::ensure!(
        rate >= 0.90,
        "composed-pool admission rate {rate:.2} over N={n} fell below the 90% floor \
         ({admitted}/{n} admitted)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rows() -> Vec<GenRow> {
        vec![
            GenRow {
                d: chrono::NaiveDate::from_ymd_opt(2024, 3, 1).expect("valid date"),
                id: 900,
                val: 10,
            },
            GenRow {
                d: chrono::NaiveDate::from_ymd_opt(2024, 3, 2).expect("valid date"),
                id: 5000,
                val: 1,
            },
        ]
    }

    /// `composed_delta_values_sql_is_byte_identical_for_duckdb_and_spark`
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7,
    /// Gap 2): the dialect-aware rewrite must reproduce today's hand-rolled
    /// `(VALUES …) AS t(id, d, val)` shape EXACTLY for DuckDB (and Spark,
    /// which shares DuckDB's arm in `row_set::build_row_set_table`) — the
    /// standing DuckDB gate and the Spark leg's own re-run both depend on
    /// this staying unchanged.
    #[test]
    fn composed_delta_values_sql_is_byte_identical_for_duckdb_and_spark() {
        let rows = sample_rows();
        let duckdb = composed_delta_values_sql(smelt_core::config::BackendType::DuckDB, &rows);
        let spark = composed_delta_values_sql(smelt_core::config::BackendType::Spark, &rows);
        assert_eq!(duckdb, spark);
        assert_eq!(
            duckdb,
            "(VALUES (900, DATE '2024-03-01', 10), (5000, DATE '2024-03-02', 1)) AS t(id, d, val)",
            "DuckDB/Spark's row set must stay byte-identical to the pre-Phase-7 hand-rolled shape"
        );
    }

    /// `composed_delta_values_sql_under_bigquery_dialect_has_no_values_table_constructor`
    /// (plan Phase 7, Gap 2 TDD test): GoogleSQL rejects a `(VALUES …)`
    /// table-value constructor in `FROM` position (`400 Syntax error:
    /// Expected keyword JOIN but got ","`, measured live against BigQuery)
    /// — the BigQuery-dialect row set must contain no `VALUES` at all and
    /// must be the portable chained `UNION ALL` rewrite instead.
    #[test]
    fn composed_delta_values_sql_under_bigquery_dialect_has_no_values_table_constructor() {
        let rows = sample_rows();
        let bq = composed_delta_values_sql(smelt_core::config::BackendType::BigQuery, &rows);
        assert!(
            !bq.to_uppercase().contains("VALUES"),
            "BigQuery's route-3 delta row set must not use a VALUES table-value \
             constructor, got: {bq:?}"
        );
        assert!(
            bq.contains("UNION ALL"),
            "expected a chained UNION ALL rewrite, got: {bq:?}"
        );
        assert_eq!(
            bq,
            "(SELECT 900 AS id, DATE '2024-03-01' AS d, 10 AS val UNION ALL \
             SELECT 5000, DATE '2024-03-02', 1) AS t"
        );
    }

    /// `composed_route3_delta_sql_is_byte_identical_for_duckdb_under_the_staged_query_shape`
    /// (plan Phase 7, Gap 2): the full delta query (row set plus the
    /// `SELECT id, MAX(d) ... GROUP BY id` wrapper) staged into
    /// `run_windowed_keyed_maintenance`'s `compile_step` must stay
    /// byte-identical for DuckDB post-fix.
    #[test]
    fn composed_route3_delta_sql_is_byte_identical_for_duckdb_under_the_staged_query_shape() {
        let rows = sample_rows();
        let sql = composed_route3_delta_sql(smelt_core::config::BackendType::DuckDB, &rows);
        assert_eq!(
            sql,
            "SELECT id, MAX(d) AS last_seen FROM (VALUES (900, DATE '2024-03-01', 10), \
             (5000, DATE '2024-03-02', 1)) AS t(id, d, val) GROUP BY id"
        );
    }
}
