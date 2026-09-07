//! Real-fixture, DuckDB-backed coverage for key temporal locality's **route
//! 2 derived sub-route** (`docs/specs/incremental_shapes.md` §"Key temporal
//! locality (the time-partitioned output)"; `docs/outcomes/
//! 20260904-decision-residue/outcome.md` phase 3).
//!
//! Stages a keyed model (`unique_key: [id, d]`, `pdate = CAST(d AS DATE)`,
//! `timeseries.partition_column: pdate`) with **no** declared
//! `functional_dependencies:` at all, derives the plan through the real
//! Salsa path (`smelt_db::maintenance_plan_report`), and asserts it is
//! admitted with `LocalitySlice::DeltaValues` — the derived sub-route
//! (`smelt_logical::analysis::key_derived`), not the declared fallback.
//!
//! `pdate`'s projection (`CAST(d AS DATE)`, not a literal `GROUP BY` column
//! text and not a `COALESCE`-shaped once-write spelling) is refused by
//! `classify_cumulative`'s runtime grammar independently of locality
//! admission — the same pre-existing gap `ComposedKeyedRecipe`'s own doc
//! comment documents for `KeyDetermined`. This file therefore drives the
//! windowed-keyed-maintenance driver
//! (`smelt_runtime::maintenance_driver::run_windowed_keyed_maintenance`)
//! directly against a real `DuckDbBackend`, exactly as `crates/
//! smelt-cli/tests/maintenance_conformance/gate.rs`'s own
//! `drive_composed_derived_and_assert` does — proving the actual merge
//! mechanics run and equal a full-refresh oracle, not just admission.

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::{AggregatorColumn, CrossPartitionCombiner, CumulativeClassification};
use smelt_maintenance_testkit::recipe::{ComposedKeyedRecipe, ComposedRoute};
use smelt_maintenance_testkit::render;
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "locality-route2-derived-test",
        model_name: "locality-route2-derived-test",
        reporter: &NO_OP_REPORTER,
    }
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — mirrors `crates/smelt-cli/tests/maintenance_conformance/
/// gate.rs::classify_composed_full` (duplicated rather than imported: that
/// function lives in a `smelt-cli` test binary, which cannot be depended on
/// from `smelt-runtime`).
fn classify_composed(
    project_dir: &std::path::Path,
    model_name: &str,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project_dir.join(format!("models/{model_name}.sql"));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project_dir.to_path_buf(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file =
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf());
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
            "staged model {model_name:?} (expected at {}) not found among discovered models",
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

fn classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string(), "d".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "total".to_string(),
            per_partition_agg: "SUM".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: smelt_logical::DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "d".to_string(),
                partition_column: "d".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    }
}

fn oracle_sql(source_name: &str) -> String {
    format!(
        "SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM main.sources_{source_name} \
         GROUP BY id, d"
    )
}

async fn assert_multiset_equal(backend: &DuckDbBackend, model_name: &str, source_name: &str) {
    let maintained = backend
        .execute_sql(&format!("SELECT * FROM main.{model_name}"))
        .await
        .expect("query maintained table");
    let oracle = backend
        .execute_sql(&oracle_sql(source_name))
        .await
        .expect("query oracle");
    let mut maintained_rows: Vec<Vec<(String, String)>> =
        smelt_runtime::check_runner::batches_to_rows(&maintained)
            .into_iter()
            .map(|m| m.into_iter().collect())
            .collect();
    let mut oracle_rows: Vec<Vec<(String, String)>> =
        smelt_runtime::check_runner::batches_to_rows(&oracle)
            .into_iter()
            .map(|m| m.into_iter().collect())
            .collect();
    maintained_rows.sort();
    oracle_rows.sort();
    assert_eq!(
        maintained_rows, oracle_rows,
        "maintained table must equal the full-refresh oracle"
    );
}

/// The derived sub-route admits with no declaration, and its actual merge
/// mechanics (driven directly, since `classify_cumulative` refuses the
/// scalar-wrapper projection independently of locality admission) equal
/// the full-refresh oracle after two windows.
#[tokio::test]
async fn key_derived_partition_model_admits_and_runs() {
    let recipe = ComposedKeyedRecipe::new(ComposedRoute::KeyDerived);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("dev.duckdb");
    std::fs::create_dir_all(&project_dir).expect("create project dir");
    let project =
        render::stage_composed(&recipe, &project_dir, &db_path).expect("stage composed recipe");

    // No `functional_dependencies:` anywhere in the staged model file — the
    // derived sub-route's whole point.
    let staged_model =
        std::fs::read_to_string(project_dir.join(format!("models/{}.sql", recipe.model_name)))
            .expect("read staged model");
    assert!(
        !staged_model.contains("functional_dependencies"),
        "the derived sub-route must admit with no declared FD: {staged_model}"
    );

    let (plan, diags) = classify_composed(&project.project_dir, &recipe.model_name)
        .expect("classify staged recipe");
    let plan = plan.unwrap_or_else(|| panic!("no plan derived: diagnostics={diags:#?}"));
    assert!(
        plan.refusals.is_empty(),
        "the derived sub-route must admit with no refusals: {:?}",
        plan.refusals
    );
    let key_locality = plan
        .key_locality
        .as_ref()
        .expect("plan must carry key_locality");
    assert!(
        matches!(key_locality.slice, LocalitySlice::DeltaValues { .. }),
        "the derived sub-route must admit LocalitySlice::DeltaValues, got {:?}",
        key_locality.slice
    );

    let backend = DuckDbBackend::new(&project.db_path, "main")
        .await
        .expect("open backend");
    let classification = classification(&recipe);
    let suppression = smelt_logical::maintenance::choice::WriteSuppression::Suppressed {
        compared_columns: vec!["total".to_string()],
    };

    // Window 1: two rows for the same key on different dates.
    backend
        .execute_sql(&format!(
            "INSERT INTO main.sources_{} VALUES (DATE '2026-01-01', 1, 10), (DATE '2026-01-02', 1, 5)",
            recipe.source.name
        ))
        .await
        .expect("insert window 1 rows");
    let compile_step_1 = move |step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
        Ok(format!(
            "SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM \
             (VALUES (1, DATE '2026-01-01', 10), (1, DATE '2026-01-02', 5)) AS t(id, d, val) \
             WHERE d = DATE '{}' GROUP BY id, d",
            step.partition_value
        ))
    };
    let steps_1 = driving_steps(
        "2026-01-01",
        "2026-01-03",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps 1");
    run_windowed_keyed_maintenance(
        &backend,
        &recipe.model_name,
        "main",
        &recipe.model_name,
        &steps_1,
        &classification,
        None,
        &suppression,
        None,
        compile_step_1,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("window 1 merge must succeed");
    assert_multiset_equal(&backend, &recipe.model_name, &recipe.source.name).await;

    // Window 2: a new key/date pair — a distinct `(id, d)` row the first
    // window never touched.
    backend
        .execute_sql(&format!(
            "INSERT INTO main.sources_{} VALUES (DATE '2026-01-03', 2, 7)",
            recipe.source.name
        ))
        .await
        .expect("insert window 2 rows");
    let compile_step_2 = move |step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
        Ok(format!(
            "SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM \
             (VALUES (2, DATE '2026-01-03', 7)) AS t(id, d, val) \
             WHERE d = DATE '{}' GROUP BY id, d",
            step.partition_value
        ))
    };
    let steps_2 = driving_steps(
        "2026-01-03",
        "2026-01-04",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps 2");
    run_windowed_keyed_maintenance(
        &backend,
        &recipe.model_name,
        "main",
        &recipe.model_name,
        &steps_2,
        &classification,
        None,
        &suppression,
        None,
        compile_step_2,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("window 2 merge must succeed");
    assert_multiset_equal(&backend, &recipe.model_name, &recipe.source.name).await;
}
