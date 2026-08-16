//! Phase 7 (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`):
//! lowering + execution of a key-addressed model-edge cell
//! (`docs/specs/incremental_models.md` §"Upstream model edges"). A clockless
//! `KeyedUpsert` upstream model folds into its downstream via the repair
//! family's own `Technique::PerGroupRecompute`, restricted to the upstream's
//! affected key set (the group-grain fingerprint sidecar diff over the
//! upstream's own output table) rather than a partition-interval scan.
//!
//! Legs 1–5 need no backend (pure resolution / emitter unit tests); leg 6
//! drives a real two-model chain through `execute_project` against a real
//! DuckDB backend.

use std::collections::HashSet;

use smelt_dialect::SqlDialect;
use smelt_logical::analysis::output_delta::OutputDelta;
use smelt_logical::maintenance::derive::ModelEdge;
use smelt_runtime::maintenance_driver::resolve_live_key_addressed_model_edge_cell;

/// The downstream model this file's unit legs share: `grain: key`,
/// `unique_key: user_id`, reading the upstream's own `total` column without
/// renaming — the common case where the downstream's own grain columns
/// literally are the upstream's key columns.
const DOWNSTREAM_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: user_id\n\
     ---\n";

const DOWNSTREAM_MODEL_SQL: &str = "SELECT user_id, total FROM smelt.models.agg";

fn metadata_and_sql(text: &str) -> (smelt_core::ModelMetadata, String) {
    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(text).expect("parse frontmatter")
    else {
        panic!("single-model file");
    };
    (*metadata, text[sql_offset..].to_string())
}

fn keyed_edge(name: &str, keys: &[&str]) -> ModelEdge {
    ModelEdge {
        name: name.to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: keys.iter().map(|s| s.to_string()).collect(),
        }),
    }
}

// ── 1 ────────────────────────────────────────────────────────────────────
#[test]
fn key_addressed_cell_resolves_live_from_the_real_plan() {
    let text = format!("{DOWNSTREAM_MODEL_FILE}{DOWNSTREAM_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let edges = vec![keyed_edge("agg", &["user_id"])];

    let resolved = resolve_live_key_addressed_model_edge_cell(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
    )
    .expect("resolution must not error")
    .expect("a live key-addressed cell must resolve");

    let (edge_name, cell, key_scope, upstream_keys, digest_columns, _write) = resolved;
    assert_eq!(edge_name, "agg");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::PerGroupRecompute
    );
    assert_eq!(key_scope.keys, vec!["user_id".to_string()]);
    assert_eq!(key_scope.from, "agg");
    assert_eq!(upstream_keys, vec!["user_id".to_string()]);
    assert!(
        !digest_columns.is_empty(),
        "the digest column set must never be empty — it is the group-grain sidecar's own hash \
         input"
    );
}

// ── 1b (phase 2 task: grain: partition characterization) ──────────────────
/// `docs/outcomes/20260816-scheduler-delta-signatures/phases/02-plan.md`
/// test 3: the SAME derivation resolves identically for a `grain:
/// partition` downstream with no declared `unique_key` — a constant `d`
/// projected (never in `GROUP BY`, since it is trivially single-valued per
/// group) and `GROUP BY user_id` alone, proving the downstream's row
/// identity from the walk rather than a declared key. Pins that the
/// `grain: key` / `grain: partition` gap phase 2 closes was ALWAYS
/// dispatch-only — plan derivation (`resolve_live_key_addressed_model_edge_cell`)
/// never depended on the downstream's own declared grain in the first
/// place.
#[test]
fn key_addressed_cell_resolves_for_a_partition_grain_downstream() {
    let text = "---\n\
         materialization: table\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         refresh: incremental\n\
         grain: partition\n\
         ---\n\
         SELECT DATE '2024-01-01' AS d, user_id, ANY_VALUE(total) AS total FROM \
         smelt.models.agg GROUP BY user_id\n";
    let (metadata, sql) = metadata_and_sql(text);
    let edges = vec![keyed_edge("agg", &["user_id"])];

    let resolved = resolve_live_key_addressed_model_edge_cell(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
    )
    .expect("resolution must not error")
    .expect("a live key-addressed cell must resolve for a grain: partition downstream too");

    let (edge_name, cell, key_scope, upstream_keys, digest_columns, _write) = resolved;
    assert_eq!(edge_name, "agg");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::PerGroupRecompute
    );
    assert_eq!(
        key_scope.keys,
        vec!["user_id".to_string()],
        "the same key_scope as the grain: key leg (`key_addressed_cell_resolves_live_from_the_\
         real_plan`) — the downstream's own declared grain never entered this proof"
    );
    assert_eq!(key_scope.from, "agg");
    assert_eq!(upstream_keys, vec!["user_id".to_string()]);
    assert!(
        !digest_columns.is_empty(),
        "the digest column set must never be empty — it is the group-grain sidecar's own hash \
         input"
    );
}

// ── 2 ────────────────────────────────────────────────────────────────────
#[test]
fn missing_key_scope_column_on_the_upstream_fails_loud() {
    // The downstream renames the key it reads (`AS uid`) — its own proven
    // grain column is `uid`, which the upstream relation (whose real key
    // column is `user_id`) does not carry.
    let text =
        format!("{DOWNSTREAM_MODEL_FILE}SELECT user_id AS uid, total FROM smelt.models.agg\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let edges = vec![keyed_edge("agg", &["user_id"])];

    // A renamed key column does not resolve the model's own grain through
    // `user_id` at all in `admit_key_addressed_recompute`'s proof, so this
    // either refuses admission (no cell resolves) or — if it did resolve —
    // must fail loud rather than silently querying `uid` on the upstream
    // table. Assert the actually-reachable shape: no live cell (the
    // narrower, and today's real, outcome), never a panic or a wrong-but-
    // silent success.
    let result = resolve_live_key_addressed_model_edge_cell(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
    );
    match result {
        Ok(None) => {}
        Ok(Some(_)) => panic!(
            "a renamed key column must not resolve a live key-addressed cell that would query \
             the upstream by the wrong name"
        ),
        Err(e) => {
            assert!(
                e.to_string().contains("MaintenanceKeyScopeColumnMissing"),
                "expected a MaintenanceKeyScopeColumnMissing refusal, got: {e}"
            );
        }
    }
}

// ── 3 ────────────────────────────────────────────────────────────────────
#[test]
fn non_duckdb_dialect_refuses_key_addressed_discovery() {
    let text = format!("{DOWNSTREAM_MODEL_FILE}{DOWNSTREAM_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let edges = vec![keyed_edge("agg", &["user_id"])];

    let err = resolve_live_key_addressed_model_edge_cell(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::SparkSQL,
    )
    .expect_err("a non-DuckDB dialect must refuse before any backend call");
    assert!(
        err.to_string().contains("not supported") || err.to_string().contains("Spark SQL"),
        "expected an unsupported-dialect refusal, got: {err}"
    );
}

// ── 4 ────────────────────────────────────────────────────────────────────
#[test]
fn affected_keys_select_restricts_to_the_changed_upstream_keys() {
    let sql = smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select(
        "main.agg",
        &["user_id".to_string()],
        &["user_id".to_string()],
        &["1".to_string(), "2".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert!(
        sql.contains("IN ('1', '2')"),
        "expected the changed-key literal list in the WHERE clause, got: {sql}"
    );
    assert!(
        sql.starts_with("SELECT DISTINCT"),
        "expected a DISTINCT projection over the downstream's own key columns, got: {sql}"
    );
    assert!(
        !sql.to_uppercase().contains("SELECT DISTINCT * "),
        "must never be an unrestricted scan: {sql}"
    );
}

#[test]
fn affected_keys_select_is_a_well_typed_empty_relation_for_no_changed_keys() {
    let sql = smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select(
        "main.agg",
        &["user_id".to_string()],
        &["user_id".to_string()],
        &[],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert!(
        sql.contains("WHERE FALSE"),
        "an empty changed-key set must yield a well-typed empty relation, got: {sql}"
    );
}

// ── 5 (real DuckDB end-to-end chain) ───────────────────────────────────
mod chain {
    use std::sync::Arc;

    fn write(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn stage_chain_project(project_dir: &std::path::Path) {
        write(
            project_dir,
            "smelt.yml",
            "name: key_addressed_chain\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    schema: main\n\
             default_materialization: view\n",
        );
        write(
            project_dir,
            "models/sources/payments.yml",
            "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
             - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
        write(
            project_dir,
            "models/agg.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, ANY_VALUE(total) AS total FROM smelt.agg GROUP BY user_id\n",
        );
    }

    /// Same two-model chain as [`stage_chain_project`], except `downstream`
    /// is `grain: partition` (a constant `d` projected but NOT in `GROUP
    /// BY`, `GROUP BY user_id` alone — the `DagBody::PartitionOverKeyedId`
    /// shape from `crates/smelt-maintenance-testkit/src/dag.rs::
    /// keyed_partition_sink_dag`) rather than `grain: key`. A declared
    /// `unique_key: user_id` alongside `grain: partition` trips
    /// `GrainAssertionMismatch`, so the walk proves the downstream's own
    /// row identity includes `user_id` from `GROUP BY` alone, and
    /// `admit_key_addressed_recompute`'s grain proof resolves through
    /// `user_id` — the same shape phase 7 flagged as deriving an admitted
    /// but undispatched key-addressed cell. `d` is deliberately left OUT of
    /// `GROUP BY` (a literal projection is trivially single-valued per
    /// group): grouping by `d`'s own output ALIAS instead of leaving it out
    /// would fail the walk's grain proof closed for the whole scope, since
    /// `analysis::walk::group_by_output_keys` matches a grouping key
    /// against a select item's own expression text, not its alias — see
    /// `DagBody::PartitionOverKeyedId`'s own render comment for the same
    /// note.
    fn stage_chain_project_partition_downstream(project_dir: &std::path::Path) {
        write(
            project_dir,
            "smelt.yml",
            "name: key_addressed_chain\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    schema: main\n\
             default_materialization: view\n",
        );
        write(
            project_dir,
            "models/sources/payments.yml",
            "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
             - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
        write(
            project_dir,
            "models/agg.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\ntimeseries:\n  event_time_column: d\n  \
             partition_column: d\n  granularity: day\nrefresh: incremental\n\
             grain: partition\n---\n\
             SELECT DATE '2024-01-01' AS d, user_id, ANY_VALUE(total) AS total \
             FROM smelt.agg GROUP BY user_id\n",
        );
    }

    /// Same as [`stage_chain_project_partition_downstream`], except
    /// `downstream` reads a SECOND inbound ref — a clocked declared source
    /// (`flags`) the key-addressed model-edge cell does not cover (it only
    /// restricts the `agg` model edge's affected keys, never a declared
    /// source). Pins the substitution gate (phase 2 task 5): a
    /// partition-grain downstream with an uncovered input must keep its
    /// ordinary route rather than risk silently dropping the uncovered
    /// component (composed multi-component dispatch is phase 3's scope).
    /// `flags` carries every `user_id` `payments` does, so the added `WHERE
    /// ... IN` join changes no row — the test isolates the substitution
    /// gate from any join-semantics correctness question.
    fn stage_chain_project_partition_downstream_with_uncovered_source(
        project_dir: &std::path::Path,
    ) {
        write(
            project_dir,
            "smelt.yml",
            "name: key_addressed_chain\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    schema: main\n\
             default_materialization: view\n",
        );
        write(
            project_dir,
            "models/sources/payments.yml",
            "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
             - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
        write(
            project_dir,
            "models/sources/flags.yml",
            "description: flags\ncolumns:\n- name: user_id\n  type: INTEGER\n\
             - name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
        write(
            project_dir,
            "models/agg.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\ntimeseries:\n  event_time_column: d\n  \
             partition_column: d\n  granularity: day\nrefresh: incremental\n\
             grain: partition\n---\n\
             SELECT DATE '2024-01-01' AS d, a.user_id, ANY_VALUE(a.total) AS total \
             FROM smelt.agg a WHERE a.user_id IN (SELECT user_id FROM smelt.sources.flags) \
             GROUP BY a.user_id\n",
        );
    }

    async fn seed_flags(backend: &dyn smelt_backend::Backend) {
        backend
            .execute_sql("CREATE TABLE main.sources_flags (user_id INTEGER, d DATE)")
            .await
            .expect("create flags source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_flags VALUES \
                 (1, DATE '2025-01-01'), (2, DATE '2025-01-01')",
            )
            .await
            .expect("seed flags");
    }

    async fn seed_payments(backend: &dyn smelt_backend::Backend) {
        backend
            .execute_sql(
                "CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), \
                 d DATE)",
            )
            .await
            .expect("create payments source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_payments VALUES \
                 (1, 100.00, DATE '2025-01-01'), (1, 50.00, DATE '2025-01-02'), \
                 (2, 70.00, DATE '2025-01-01')",
            )
            .await
            .expect("seed payments");
    }

    fn build_db_and_graph(
        project_dir: &std::path::Path,
        config: &smelt_core::config::Config,
    ) -> (
        Arc<tokio::sync::Mutex<smelt_db::Database>>,
        Arc<tokio::sync::Mutex<smelt_core::graph::DependencyGraph>>,
    ) {
        use smelt_core::ModelDiscovery;
        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
        let sql_models = discovery.discover_models().expect("discover_models");

        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(project_dir.to_path_buf(), String::new());
        let source_files: Vec<_> = sql_models
            .iter()
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);
        db.set_active_target(
            config
                .target
                .clone()
                .map(|t| std::sync::Arc::from(t.as_str())),
        );

        let graph =
            smelt_core::graph::DependencyGraph::build(sql_models, None).expect("build graph");

        (
            Arc::new(tokio::sync::Mutex::new(db)),
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
    }

    fn select_request(models: &[&str]) -> smelt_runtime::types::ExecuteRequest {
        smelt_runtime::types::ExecuteRequest {
            target: "dev".to_string(),
            select: models.iter().map(|s| s.to_string()).collect(),
            exclude: vec![],
            start: None,
            end: None,
            batch_size_days: None,
            per_partition: false,
            full_refresh: false,
            dry_run: false,
            enforce_safety: false,
            allow_column_removal: false,
            allow_full_refresh: false,
            ephemeral_seed_ctes: vec![],
            run_checks: false,
            checks: vec![],
            jobs: None,
            retry_max: None,
            retry_backoff_ms: None,
            resume: false,
            technique_overrides: vec![],
        }
    }

    struct DuckDbBackendFactory {
        db_path: std::path::PathBuf,
    }

    impl smelt_runtime::execute::BackendFactory for DuckDbBackendFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            target_config: &'a smelt_core::config::Target,
            _project_dir: &'a std::path::Path,
        ) -> smelt_runtime::execute::BackendFuture<'a> {
            let path = self.db_path.clone();
            let schema = target_config.schema.clone();
            Box::pin(async move {
                let backend = smelt_backend_duckdb::DuckDbBackend::new(&path, &schema)
                    .await
                    .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
                Ok(Box::new(backend) as Box<dyn smelt_backend::Backend>)
            })
        }
    }

    async fn scalar_text(backend: &dyn smelt_backend::Backend, sql: &str) -> String {
        let batches = backend.execute_sql(sql).await.expect("query");
        let batch = batches.first().expect("one batch");
        assert_eq!(batch.num_rows(), 1, "expected exactly one row for: {sql}");
        let col = batch.column(0);
        arrow::util::display::array_value_to_string(col, 0).expect("render value")
    }

    // ── 6 ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn keyed_chain_maintains_only_the_changed_keys_end_to_end() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_chain_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments(&backend).await;
        }

        // Run 1: creation. Both `agg` and `downstream` materialize via their
        // own fold path — there is nothing to repair yet.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "chain-run-1".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("first run (create) must succeed");
        }

        let full_refresh_downstream_user_1 = {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            scalar_text(
                &backend,
                "SELECT total FROM main.downstream WHERE user_id = 1",
            )
            .await
        };
        assert_eq!(full_refresh_downstream_user_1, "150.00");

        // Mutate user 1's contribution in place — user 2 is untouched.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND amount = 100.00")
                .await
                .expect("mutate payments");
        }

        // Run 2: `agg` re-folds via its own snapshot-reconcile path;
        // `downstream` resolves a live key-addressed model-edge cell and
        // recomputes only user 1's group.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "chain-run-2".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second run (key-addressed recompute) must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_eq!(
                record.strategy, "per_group_recompute",
                "the upstream's key-addressed fold must dispatch the repair family, not a whole-\
                 table reconcile"
            );
        }

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");

        let repaired = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(
            repaired, "250.00",
            "user 1's group must reflect the mutated contribution (50.00 + 200.00)"
        );
        let untouched = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 2",
        )
        .await;
        assert_eq!(
            untouched, "70.00",
            "user 2's group must be bit-identical — it was never in the affected-key set"
        );

        // Cross-check against a full-refresh oracle over the CURRENT source
        // state — the equivalence invariant this technique must uphold.
        let oracle_user_1 = scalar_text(
            &backend,
            "SELECT SUM(amount) FROM main.sources_payments WHERE user_id = 1",
        )
        .await;
        assert_eq!(oracle_user_1, repaired);
    }

    // ── 7 (dispatch outside `grain: key`) ──────────────────────────────
    /// Phase 2 (`docs/outcomes/20260816-scheduler-delta-signatures/phases/
    /// 02-plan.md`): the derived key-addressed model-edge cell must
    /// actually run for a `grain: partition` downstream of the SAME
    /// clockless `keyed upsert` upstream test 6 exercises — dispatch is
    /// keyed by the component's addressing (`docs/specs/incremental_models.md`
    /// §"Dispatch — from propagated components to run units"), never by
    /// the downstream model's own declared grain.
    #[tokio::test]
    async fn partition_grain_downstream_dispatches_key_addressed_cell() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_chain_project_partition_downstream(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments(&backend).await;
        }

        // Run 1: creation — nothing to repair yet.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "kpart-run-1".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("first run (create) must succeed");
        }

        // Mutate user 1's contribution in place — user 2 is untouched.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND amount = 100.00")
                .await
                .expect("mutate payments");
        }

        // Run 2: `downstream` (grain: partition) must resolve and dispatch
        // the SAME key-addressed model-edge cell the `grain: key` chain
        // (test 6) dispatches — the repair family, not the ordinary
        // correct-but-full route.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "kpart-run-2".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second run (key-addressed recompute) must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_eq!(
                record.strategy, "per_group_recompute",
                "a grain: partition downstream of a clockless keyed-upsert upstream must \
                 dispatch the repair family's key-addressed cell, not the ordinary route"
            );
        }

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");

        let repaired = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(
            repaired, "250.00",
            "user 1's group must reflect the mutated contribution (50.00 + 200.00)"
        );
        let untouched = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 2",
        )
        .await;
        assert_eq!(
            untouched, "70.00",
            "user 2's group must be bit-identical — it was never in the affected-key set"
        );
    }

    // ── 8 (substitution gate: an uncovered second input) ───────────────
    /// Phase 2 task 5's widen-never-narrow substitution gate: `downstream`
    /// reads BOTH the key-addressed `agg` model edge AND a declared source
    /// (`flags`) the key-addressed cell does not restrict. The non-keyed
    /// dispatch site must refuse to substitute — the model keeps its
    /// ordinary route rather than risk silently dropping the uncovered
    /// `flags` component — and the result must still be multiset-correct.
    #[tokio::test]
    async fn partition_grain_downstream_with_an_uncovered_input_keeps_the_ordinary_route() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_chain_project_partition_downstream_with_uncovered_source(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments(&backend).await;
            seed_flags(&backend).await;
        }

        // Run 1: creation.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "kpart-uncovered-run-1".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("first run (create) must succeed");
        }

        // Mutate user 1's contribution — the SAME shape test 7 exercises.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND amount = 100.00")
                .await
                .expect("mutate payments");
        }

        // Run 2: the substitution gate must refuse (an uncovered `flags`
        // ref is present), so `downstream` keeps its ordinary route.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "kpart-uncovered-run-2".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &smelt_runtime::NoOpReporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second run must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_ne!(
                record.strategy, "per_group_recompute",
                "a downstream with an uncovered second inbound ref must never substitute the \
                 key-addressed cell for its ordinary route"
            );
        }

        // Multiset-correctness is preserved regardless of which route ran.
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let repaired = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(
            repaired, "250.00",
            "user 1's group must reflect the mutated contribution (50.00 + 200.00) even via \
             the ordinary route"
        );
        let untouched = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 2",
        )
        .await;
        assert_eq!(untouched, "70.00", "user 2's group must be unchanged");
    }
}
