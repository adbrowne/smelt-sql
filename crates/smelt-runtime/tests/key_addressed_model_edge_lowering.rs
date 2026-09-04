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
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error")
    .expect("a live key-addressed cell must resolve");

    let (edge_name, cell, key_scope, upstream_keys, group_key, digest_columns, _write) = resolved;
    assert_eq!(edge_name, "agg");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::PerGroupRecompute
    );
    assert_eq!(key_scope.keys, vec!["user_id".to_string()]);
    assert_eq!(key_scope.from, "agg");
    assert_eq!(
        key_scope.discovery,
        smelt_logical::maintenance::KeyDiscovery::UpstreamKeyed
    );
    assert_eq!(upstream_keys, vec!["user_id".to_string()]);
    assert_eq!(group_key, vec!["user_id".to_string()]);
    assert!(
        !digest_columns.is_empty(),
        "the digest column set must never be empty — it is the group-grain sidecar's own hash \
         input"
    );
}

// ── 1b (Phase 11, `docs/outcomes/20260815-definition-delta-migrate/phases/
//        11-plan.md`) ──────────────────────────────────────────────────
/// A `grain: partition` + `timeseries:` downstream reading a clockless
/// `KeyedUpsert` upstream — same shape as `DOWNSTREAM_MODEL_FILE` above but
/// with a partition axis of its own, proving `resolve_live_key_addressed_
/// model_edge_cell` stays reachable from a non-keyed downstream's inputs
/// (the derivation is grain-agnostic; only the run-loop *dispatch* branch
/// was previously narrowed to `grain: key`).
const DOWNSTREAM_PARTITION_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: event_date\n\
     \x20\x20event_time_column: event_date\n\
     \x20\x20granularity: day\n\
     ---\n";

// No top-level `unique_key:` (declaring one here would derive `grain: key`
// from the shape facts, per `derive_grain`, and fail the `grain: partition`
// assertion before this ever reaches the maintenance layer) — the row
// identity `admit_key_addressed_recompute` needs is instead PROVEN from the
// SQL's own `GROUP BY user_id`, exactly as `stage_partition_chain_project`'s
// real `downstream.sql` does.
const DOWNSTREAM_PARTITION_MODEL_SQL: &str = "SELECT user_id, ANY_VALUE(total) AS total, \
     DATE '2024-01-01' AS event_date FROM smelt.models.agg GROUP BY user_id";

#[test]
fn partition_grain_downstream_resolves_the_key_addressed_cell() {
    let text = format!("{DOWNSTREAM_PARTITION_MODEL_FILE}{DOWNSTREAM_PARTITION_MODEL_SQL}\n");
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
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error")
    .expect(
        "a grain: partition downstream reading a clockless KeyedUpsert upstream must still \
         resolve a live key-addressed cell — the route is dispatched irrespective of the \
         downstream's own grain (`docs/specs/incremental_models.md` §\"Upstream model edges\")",
    );

    let (edge_name, cell, key_scope, upstream_keys, _group_key, _digest_columns, _write) = resolved;
    assert_eq!(edge_name, "agg");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::PerGroupRecompute
    );
    assert_eq!(key_scope.keys, vec!["user_id".to_string()]);
    assert_eq!(upstream_keys, vec!["user_id".to_string()]);
}

// ── 1c (Phase 24b, `docs/outcomes/20260815-definition-delta-migrate/phases/
//        24b-plan.md`) ─────────────────────────────────────────────────
/// The downstream regroups the upstream's rows onto `device_id` — a real
/// column of the upstream relation, but not the upstream's own `KeyedUpsert`
/// key (`event_id`). The grain-over-upstream route admits this and the
/// group-grain sidecar it resolves groups at the downstream's own grain
/// (`device_id`), not the upstream's key.
const DOWNSTREAM_GRAIN_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: device_id\n\
     ---\n";

const DOWNSTREAM_GRAIN_MODEL_SQL: &str =
    "SELECT device_id, SUM(amount) AS total FROM smelt.models.agg GROUP BY device_id";

#[test]
fn grain_route_groups_sidecar_at_downstream_grain() {
    let text = format!("{DOWNSTREAM_GRAIN_MODEL_FILE}{DOWNSTREAM_GRAIN_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let edges = vec![keyed_edge("agg", &["event_id"])];

    let resolved = resolve_live_key_addressed_model_edge_cell(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error")
    .expect("a grain-over-upstream cell must resolve");

    let (edge_name, cell, key_scope, upstream_keys, group_key, _digest_columns, _write) = resolved;
    assert_eq!(edge_name, "agg");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::PerGroupRecompute
    );
    assert_eq!(key_scope.keys, vec!["device_id".to_string()]);
    assert_eq!(
        key_scope.discovery,
        smelt_logical::maintenance::KeyDiscovery::DownstreamGrainOverUpstream
    );
    // The upstream's own KeyedUpsert key columns stay `event_id` — the
    // sidecar's own grouping key (`group_key`) is what differs, and it must
    // be the downstream's grain, never re-derived from `upstream_keys`.
    assert_eq!(upstream_keys, vec!["event_id".to_string()]);
    assert_eq!(group_key, vec!["device_id".to_string()]);
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
        &smelt_logical::maintenance::availability::StateAvailability::all(),
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
        &smelt_logical::maintenance::availability::StateAvailability::all(),
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

/// Phase 24b: the upstream-keyed route's own affected-keys shape
/// (`emit_key_addressed_affected_keys_select`'s forward-projection `SELECT`)
/// is byte-for-byte unchanged by the new grain-over-upstream route existing
/// alongside it — same assertions as
/// `affected_keys_select_restricts_to_the_changed_upstream_keys` above,
/// pinned again here under this phase's own name since that emitter is the
/// exact function `resolve_key_addressed_affected_keys`'s `UpstreamKeyed`
/// arm still delegates to, unmodified.
#[test]
fn equal_key_route_is_unchanged() {
    let sql = smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select(
        "main.agg",
        &["user_id".to_string()],
        &["user_id".to_string()],
        &["1".to_string(), "2".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert!(sql.starts_with("SELECT DISTINCT"));
    assert!(sql.contains("FROM main.agg"));
    assert!(sql.contains("IN ('1', '2')"));
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
             unique_key: user_id\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
             payments:\n        allow_full_scan: true\n---\n\
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

    /// Same `payments`/`agg` pair as [`stage_chain_project`], but
    /// `downstream` is declared `grain: partition` (+ its own `timeseries:`)
    /// instead of `grain: key` — all rows fall in the same single partition
    /// (`event_date`), pinned to `2025-01-01` so the run window below can
    /// cover both it and the seeded `payments` dates in the same request.
    fn stage_partition_chain_project(project_dir: &std::path::Path) {
        write(
            project_dir,
            "smelt.yml",
            "name: key_addressed_partition_chain\nversion: 1\npaths:\n  - models\n\
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
             unique_key: user_id\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
             payments:\n        allow_full_scan: true\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
             timeseries:\n  partition_column: event_date\n\
             \x20\x20event_time_column: event_date\n  granularity: day\n---\n\
             SELECT user_id, ANY_VALUE(total) AS total, DATE '2025-01-01' AS event_date \
             FROM smelt.agg GROUP BY user_id\n",
        );
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
            // `agg` (this file's clocked, `grain: key` upstream) always runs
            // unwindowed via this helper — a window-forward keyed run with
            // no window now refuses unless `--full-refresh` is set
            // (`docs/specs/incremental_shapes.md` §"The key grain"). Harmless
            // for every other model this helper selects: `full_refresh` is
            // only consulted by that one windowless-keyed-run branch and the
            // definition-delta gate (unexercised by this file), never by the
            // key-addressed model-edge dispatch this file's tests pin.
            full_refresh: true,
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

    fn windowed_select_request(
        models: &[&str],
        start: &str,
        end: &str,
    ) -> smelt_runtime::types::ExecuteRequest {
        smelt_runtime::types::ExecuteRequest {
            start: Some(start.to_string()),
            end: Some(end.to_string()),
            ..select_request(models)
        }
    }

    // Phase 11 (`docs/outcomes/20260815-definition-delta-migrate/phases/
    // 11-plan.md`): the same key-addressed model-edge chain as leg 6 above,
    // but `downstream` is `grain: partition` rather than `grain: key` — the
    // run loop must dispatch the cell on the non-keyed incremental branch
    // too, not only inside `plan_is_keyed`.
    // ── 7 ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn partition_grain_chain_maintains_only_the_changed_keys_end_to_end() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_partition_chain_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments(&backend).await;
        }

        // Run 1: creation. Both `agg` and `downstream` materialize via their
        // own fold path — there is nothing to repair yet. `agg` (`grain:
        // key`) runs unwindowed — as its own cumulative fold always does
        // (mirroring `keyed_chain_maintains_only_the_changed_keys_end_to_end`
        // above) — in a SEPARATE `execute_project` call from `downstream`
        // (`grain: partition`, which needs an explicit run window): giving
        // `agg` a window here would register its snapshot-reconcile ledger
        // entry against that window, which the SAME window on run 2 below
        // would then refuse re-folding (`KeyedReprocessedWindow`,
        // never-fold-twice) — a windowed request applies uniformly to every
        // model it selects, not per-model.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "partition-chain-run-1-agg".to_string(),
                select_request(&["agg"]),
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
            .expect("first run (create agg) must succeed");
        }
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "partition-chain-run-1-downstream".to_string(),
                windowed_select_request(&["downstream"], "2025-01-01", "2025-01-03"),
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
            .expect("first run (create downstream) must succeed");
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

        // Run 2: `agg` re-folds via its own snapshot-reconcile path (again
        // unwindowed, again its own separate `execute_project` call);
        // `downstream` — despite being `grain: partition` — resolves the
        // live key-addressed model-edge cell on the non-keyed branch and
        // recomputes only user 1's group.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "partition-chain-run-2-agg".to_string(),
                select_request(&["agg"]),
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
            .expect("second run (agg re-fold) must succeed");
        }
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "partition-chain-run-2-downstream".to_string(),
                windowed_select_request(&["downstream"], "2025-01-01", "2025-01-03"),
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
                "a grain: partition downstream fed by a clockless KeyedUpsert upstream must \
                 dispatch the repair family on the non-keyed branch, not its ordinary window-\
                 forward batch loop"
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

    // ── 8 ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn partition_grain_creation_run_does_not_take_the_key_addressed_route() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_partition_chain_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments(&backend).await;
        }

        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = smelt_runtime::execute_project(
            "partition-chain-run-creation".to_string(),
            windowed_select_request(&["agg", "downstream"], "2025-01-01", "2025-01-03"),
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
        .expect("creation run must succeed");

        let record = outcome.models.get("downstream").expect("downstream ran");
        assert_ne!(
            record.strategy, "per_group_recompute",
            "the creation run has no existing table to repair — it must materialize via the \
             ordinary fold path, never the key-addressed route (`table_exists_before_run` \
             guard)"
        );
        assert_ne!(record.strategy, "diff_patch");

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let total_user_1 = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(total_user_1, "150.00");
    }

    // Phase 24b: a grain-over-upstream chain — `downstream` regroups
    // `agg`'s rows onto `device_id`, a real column of `agg` but not `agg`'s
    // own `KeyedUpsert` key (`event_id`).
    fn stage_grain_over_upstream_chain_project(project_dir: &std::path::Path) {
        write(
            project_dir,
            "smelt.yml",
            "name: grain_over_upstream_chain\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    schema: main\n\
             default_materialization: view\n",
        );
        write(
            project_dir,
            "models/sources/events.yml",
            "description: events\ncolumns:\n- name: event_id\n  type: INTEGER\n\
             - name: device_id\n  type: VARCHAR\n- name: amount\n  type: DECIMAL(10,2)\n\
             - name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
        // `events` declares its own `timeseries:` clock, so `agg`'s driving
        // source is clocked — the window-forward shape, which refuses the
        // plain-overwrite (`ANY_VALUE`) combinator family. `MAX` (a
        // catalogued fold-family aggregator) is deterministic here anyway:
        // every `event_id` group is a singleton.
        write(
            project_dir,
            "models/agg.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: event_id\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
             events:\n        allow_full_scan: true\n---\n\
             SELECT event_id, MAX(device_id) AS device_id, MAX(amount) AS amount\n\
             FROM smelt.sources.events\nGROUP BY event_id\n",
        );
        // No clock anywhere in this chain (`agg` declares no `timeseries:`),
        // so both `agg` and `downstream` are snapshot-reconcile-shaped:
        // every non-key column must be a plain-overwrite (`ANY_VALUE`)
        // combinator, never an additive fold — matching
        // `stage_chain_project`'s own `ANY_VALUE(total)` downstream above.
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: device_id\n---\n\
             SELECT device_id, ANY_VALUE(amount) AS amount FROM smelt.agg GROUP BY device_id\n",
        );
    }

    async fn seed_events(backend: &dyn smelt_backend::Backend) {
        backend
            .execute_sql(
                "CREATE TABLE main.sources_events (event_id INTEGER, device_id VARCHAR, \
                 amount DECIMAL(10,2), d DATE)",
            )
            .await
            .expect("create events source table");
        // One event per device — each downstream group starts as a
        // singleton, so `ANY_VALUE(amount)` is deterministic both before and
        // after the move below (never picking arbitrarily among ties).
        backend
            .execute_sql(
                "INSERT INTO main.sources_events VALUES \
                 (1, 'A', 10.00, DATE '2025-01-01'), (2, 'B', 20.00, DATE '2025-01-01')",
            )
            .await
            .expect("seed events");
    }

    // ── 9 ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn moved_grain_value_repairs_both_groups() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_grain_over_upstream_chain_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_events(&backend).await;
        }

        // Run 1: creation. `device_id = 'A'` groups event 1 alone (amount
        // 10.00); `device_id = 'B'` groups event 2 alone (amount 20.00).
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "grain-over-upstream-run-1".to_string(),
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

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            assert_eq!(
                scalar_text(
                    &backend,
                    "SELECT amount FROM main.downstream WHERE device_id = 'A'"
                )
                .await,
                "10.00"
            );
            assert_eq!(
                scalar_text(
                    &backend,
                    "SELECT amount FROM main.downstream WHERE device_id = 'B'"
                )
                .await,
                "20.00"
            );
        }

        // Move event 1 from device A to a brand-new device C directly on
        // `agg`'s own output table — the group-grain sidecar `downstream`
        // diffs against is `agg`'s output, not `events`, so this is the
        // same shape a real reconcile of `agg` would have produced. Device A
        // is now vacated (0 members); device C is a newly arriving group;
        // device B is untouched.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.agg SET device_id = 'C' WHERE event_id = 1")
                .await
                .expect("move event 1 to device C");
        }

        // Run 2: only `downstream` — `agg` is not re-run. `downstream`
        // resolves a grain-over-upstream key-addressed cell and must
        // recompute BOTH device A (vacated) and device C (arriving).
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "grain-over-upstream-run-2".to_string(),
                select_request(&["downstream"]),
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
            .expect("second run (grain-over-upstream recompute) must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_eq!(
                record.strategy, "per_group_recompute",
                "the grain-over-upstream fold must dispatch the repair family"
            );
        }

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let batches = {
            use smelt_backend::Backend;
            backend
                .execute_sql("SELECT amount FROM main.downstream WHERE device_id = 'A'")
                .await
                .expect("query device A")
        };
        let device_a_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(
            device_a_rows, 0,
            "device A must be vacated — its only member (event 1) moved to device C, and the \
             group's own recompute must actually delete the now-memberless row rather than \
             leaving a stale one behind"
        );
        let amount_c = scalar_text(
            &backend,
            "SELECT amount FROM main.downstream WHERE device_id = 'C'",
        )
        .await;
        assert_eq!(
            amount_c, "10.00",
            "device C must reflect event 1's arrival — the arriving group's own recompute"
        );
        let amount_b = scalar_text(
            &backend,
            "SELECT amount FROM main.downstream WHERE device_id = 'B'",
        )
        .await;
        assert_eq!(
            amount_b, "20.00",
            "device B's own group is bit-identical — it was never in the affected-key set"
        );
    }
}
