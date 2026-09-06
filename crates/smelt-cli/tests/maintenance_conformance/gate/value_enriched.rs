//! The closure-pruned column-scoped `MERGE` pool (`ValueEnrichedRecipe`).

use super::support::snapshot_table_rows;
use smelt_logical::maintenance::{Corner, Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::ValueEnrichedRecipe;
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{read_source_snapshot, GenRow};

// ---------------------------------------------------------------------
// `docs/plans/20260809-sensitivity-precision.md` Phase 5: the
// closure-pruned column-scoped `MERGE` pool (`ValueEnrichedRecipe`).
// ---------------------------------------------------------------------

/// Ids seeded into the staged dimension table, wide enough to cover the
/// fixed set of ids this fixed-shape test drives by hand (mirrors
/// [`KEYED_ENRICHED_DIM_SEED_MAX_ID`]'s convention, scaled down since this
/// test drives a fixed schedule rather than a generated one).
pub(crate) const VALUE_ENRICHED_DIM_SEED_MAX_ID: i64 = 20;

/// Stage a [`ValueEnrichedRecipe`] into a fresh temp project + DuckDB file —
/// the closure-pruned-enrichment-pool counterpart of
/// [`stage_keyed_enriched_recipe`]: writes both source YAMLs + the model
/// file, creates both physical source tables, and pre-seeds the dimension
/// with one row per id in `1..=VALUE_ENRICHED_DIM_SEED_MAX_ID`
/// (`attr = id * 100`).
pub(crate) fn stage_value_enriched_recipe(
    recipe: &ValueEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    // `smelt.yml`, NOT the SQL frontmatter, carries this recipe's
    // `models.<name>.merge_key:` — the top-level replacement for the retired
    // `batched.unique_key` sub-block (`docs/specs/models.md` §"Batched
    // sub-block retirement"), and the only surface for
    // `PartitionGrainConfig.unique_key` under `grain: partition`
    // (`ValueEnrichedRecipe::model_file`'s own doc comment explains why the
    // SQL-frontmatter form can't carry it — `merge_key:` never confers
    // identity). This is the column-scoped `MERGE`'s own `ON`-predicate key
    // (`decide_column_merge_dispatch`'s `model_declares_unique_key`
    // precondition) — without it the live `ColumnScopedMerge` cell resolves
    // in the derived plan but never actually dispatches at execution time.
    let smelt_yml = format!(
        "{base}models:\n  {model}:\n    merge_key: [{id}]\n",
        base = render::render_smelt_yml(&db_path),
        model = recipe.model_name,
        id = recipe.fact.key_column,
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml)?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=VALUE_ENRICHED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`ValueEnrichedRecipe`]'s staged fact source table.
pub(crate) fn insert_fact_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ),
        [],
    )?;
    Ok(())
}

/// Update a [`ValueEnrichedRecipe`]'s staged dimension row's `attr` column —
/// the value-mutation window this recipe's whole point is: the model DOES
/// select `attr` directly, so this must become visible in the maintained
/// output through the column-scoped `MERGE`, never a recompute fallback.
pub(crate) fn update_dim_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a [`ValueEnrichedRecipe`]'s staged dimension row — the
/// departed-dimension-row window: since the join is `LEFT JOIN` and closed
/// (never membership-sensitive for this shape), the fact row must SURVIVE
/// with `attr` re-derived to NULL, never disappear from the output.
pub(crate) fn delete_dim_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`ValueEnrichedRecipe`] through the real maintenance
/// derivation — the closure-pruned-enrichment-pool counterpart of
/// [`classify_keyed_enriched_full`], going through
/// `smelt_db::maintenance_plan_report`/`file_diagnostics` (the SAME
/// Salsa-backed derivation the LSP/CLI diagnostics use), not a hand-built
/// `ModelInputs` (unlike
/// `smelt-logical/tests/maintenance_tracer.rs::closed_outer_enrichment_join_upstream_mutation_derives_column_scoped_merge`,
/// which this test's plan-shape assertion mirrors end-to-end).
pub(crate) fn classify_value_enriched_full(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
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
            "staged value-enriched-pool model {:?} (expected at {}) not found among discovered \
             models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// The end-state equivalence assertion for a [`ValueEnrichedRecipe`] — the
/// closure-pruned-enrichment-pool counterpart of
/// [`assert_keyed_enriched_equivalence`]. The column-scoped `MERGE` this
/// recipe dispatches through recomputes every existing key's `attr` column
/// every run (the accepted-full-scan corner), so equivalence holds
/// unconditionally after every window, exactly like the membership-recompute
/// counterpart.
pub(crate) async fn assert_value_enriched_equivalence(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "value-enriched end-state equivalence violated for model {:?} at run {k}: \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// The fixed dimension key this test's hand-built windows exercise —
/// pre-seeded (`VALUE_ENRICHED_DIM_SEED_MAX_ID`) so its initial fact row
/// admits with a real, non-NULL `attr` before any mutation.
pub(crate) const VALUE_ENRICHED_TEST_ID: i64 = 7;

/// `value_enriched_recipe_executes_column_scoped_merge`
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 5): the derived
/// plan for a closure-pruned `LEFT JOIN` enrichment carries
/// `Technique::ColumnScopedMerge`/`Corner::ColumnMerge` for its dimension's
/// `UpstreamMutation` cell (never `Technique::DeleteInsert` — the
/// membership-recompute family a still-open closure would fall back to),
/// and driving the recipe through the real `execute_project` pipeline
/// against a real DuckDB actually DISPATCHES that technique
/// (`RunOutcome.models[..].strategy == "column_scoped_merge"`, the same
/// observable `keyed_enriched_pool_upholds_equivalence_under_dim_mutation`
/// uses to distinguish `delete_insert_suppressed` from a silent recompute
/// fallback) across a dimension VALUE mutation, a dimension ROW DELETION
/// (the departed-dimension-row case: the fact row survives with `attr`
/// re-derived to NULL, since the join never drops rows), and a zero-change
/// redelivery — matching the independently-staged full-refresh oracle at
/// every step.
#[test]
fn value_enriched_recipe_executes_column_scoped_merge() {
    let recipe = ValueEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_value_enriched_recipe(&recipe, &tmp).expect("stage value-enriched recipe");

    // --- (a) Plan-shape assertion: ColumnScopedMerge, never DeleteInsert.
    let (plan, diagnostics) =
        classify_value_enriched_full(&project, &recipe).expect("classify value-enriched recipe");
    let plan = plan.unwrap_or_else(|| {
        panic!(
            "maintenance_plan_report must return a plan for the staged recipe; diagnostics: \
             {diagnostics:#?}"
        )
    });
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.severity == smelt_db::DiagnosticSeverity::Error),
        "the staged value-enriched recipe must produce zero Error diagnostics: {diagnostics:#?}"
    );

    let dim_source = recipe.dimension.name.clone();
    let attr_cell = plan
        .cells
        .iter()
        .find(|c| {
            matches!(&c.trigger, Trigger::UpstreamMutation { source } if source == &dim_source)
                && c.group == "{attr}"
        })
        .unwrap_or_else(|| {
            panic!("no {{attr}} UpstreamMutation({dim_source}) cell in derived plan: {plan:#?}")
        });
    assert_eq!(
        attr_cell.technique,
        Technique::ColumnScopedMerge,
        "the closure-pruned LEFT JOIN's own ON read must not make {{attr}} membership-\
         sensitive — expected ColumnScopedMerge, got {:?} (plan: {plan:#?})",
        attr_cell.technique
    );
    assert_eq!(attr_cell.corner, Corner::ColumnMerge);
    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::DeleteInsert),
        "a closure-pruned enrichment must never fall back to the membership-recompute family: \
         {plan:#?}"
    );

    // --- (b)-(d): drive the real pipeline and assert dispatch + equivalence.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.fact);

    rt.block_on(async {
        // Creation run: seeds the fact row this test mutates around.
        let start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
        let end = start + chrono::Duration::days(1);
        insert_fact_row_value_enriched(
            &project,
            &recipe,
            &GenRow {
                d: start,
                id: VALUE_ENRICHED_TEST_ID,
                val: Some(42),
            },
        )
        .expect("insert seed fact row");
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet("value-enriched-creation", request)
            .await
            .expect("creation run");
        let record = outcome
            .models
            .get(&recipe.model_name)
            .expect("model ran on creation");
        assert_ne!(
            record.strategy, "column_scoped_merge",
            "the creation run must not take the column-scoped MERGE path — the target doesn't \
             exist yet"
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .expect("creation-run equivalence");

        // Every subsequent window RE-TOUCHES the creation run's own
        // `[start, end)` window rather than advancing forward: the
        // column-scoped `MERGE`'s write stays scoped to exactly the run's
        // own batch window (`used_column_scoped_merge`'s doc comment in
        // `execute.rs`: "keeps the write scoped to exactly the window a
        // DELETE+INSERT would have touched"), so a mutation is only visible
        // once a run actually re-touches the window the mutated row's fact
        // lives in — mirroring a real catch-up run over an already-processed
        // partition, not a forward advance into fresh territory.
        let run_window = |label: &'static str| (label, start, end);

        // (b) Dimension VALUE mutation: `attr` changes for an already-
        // admitted row — must become visible via a real column-scoped
        // MERGE, matching the oracle.
        let (label, start, end) = run_window("dim-value-mutation");
        update_dim_row_value_enriched(&project, &recipe, VALUE_ENRICHED_TEST_ID, 900_700)
            .unwrap_or_else(|e| panic!("{label}: update dim row failed: {e}"));
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "{label}: expected the live column-scoped MERGE to dispatch, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: equivalence check failed: {e}"));
        {
            let conn = project.connect().expect("connect");
            let attr: i64 = conn
                .query_row(
                    &format!(
                        "SELECT attr FROM main.{} WHERE id = {VALUE_ENRICHED_TEST_ID}",
                        recipe.model_name
                    ),
                    [],
                    |row| row.get(0),
                )
                .expect("mutated row must exist with the new attr value");
            assert_eq!(
                attr, 900_700,
                "{label}: the value mutation must be visible through the column-scoped MERGE"
            );
        }

        // (c) Dimension ROW DELETION: a genuine departure from the dim, but
        // NOT from the maintained output (LEFT JOIN) — the fact row must
        // survive with `attr` re-derived to NULL.
        let (label, start, end) = run_window("dim-row-deletion");
        delete_dim_row_value_enriched(&project, &recipe, VALUE_ENRICHED_TEST_ID)
            .unwrap_or_else(|e| panic!("{label}: delete dim row failed: {e}"));
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "{label}: expected the live column-scoped MERGE to dispatch, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: equivalence check failed: {e}"));
        {
            let conn = project.connect().expect("connect");
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT val, attr FROM main.{} WHERE id = {VALUE_ENRICHED_TEST_ID}",
                    recipe.model_name
                ))
                .expect("prepare survivor read-back");
            let mut rows = stmt.query([]).expect("query survivor row");
            let row = rows
                .next()
                .expect("query row")
                .expect("{label}: the departed-dimension fact row must survive, not disappear");
            let val: i64 = row.get(0).expect("val");
            let attr: Option<i64> = row.get(1).expect("attr");
            assert_eq!(val, 42, "{label}: the fact's own column must be unchanged");
            assert_eq!(
                attr, None,
                "{label}: attr must re-derive to NULL once the dim row departs, since the LEFT \
                 JOIN never drops the fact row"
            );
        }

        // (d) Zero-change redelivery: idempotent — a re-run of an
        // already-caught-up window must write nothing observable.
        let (label, redelivery_start, redelivery_end) = run_window("redelivery");
        let maintained_before = {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot before redelivery")
        };
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(redelivery_start.format("%Y-%m-%d").to_string());
        request.end = Some(redelivery_end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        // Mutation-happened discrimination (`docs/specs/incremental_models.md`
        // §"When a mutation cell dispatches"): the dimension's fingerprint is
        // unchanged since the prior run's recorded baseline, so the
        // column-scoped MERGE cell is a no-op this run — the run correctly
        // falls back to the ordinary DELETE+INSERT path instead of
        // re-dispatching a MERGE with nothing to change.
        assert_eq!(
            record.strategy, "deleteinsert",
            "{label}: an unchanged dimension's UpstreamMutation cell must be a no-op, not \
             re-dispatch the column-scoped MERGE, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(redelivery_start, redelivery_end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: redelivery equivalence check failed: {e}"));

        let maintained_after = {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot after redelivery")
        };
        assert_eq!(
            maintained_before, maintained_after,
            "{label}: the redelivery run (idempotent re-merge) must write nothing observable \
             when nothing changed — the maintained table's contents must be byte-identical \
             before and after"
        );
    });
}
