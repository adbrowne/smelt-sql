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
use smelt_runtime::maintenance_driver::resolve_live_key_addressed_model_edge_cells;

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

// ── 3b (phase 4 task 1: plural resolver) ───────────────────────────────────
/// `docs/outcomes/20260816-scheduler-delta-signatures/phases/04-plan.md`
/// test 1: a downstream reading TWO clockless `keyed upsert` upstreams gets
/// a cell for EACH, not just the first — the plural resolver
/// [`resolve_live_key_addressed_model_edge_cells`] the run loop's dispatch
/// composition (phase 4 tasks 2–5) depends on.
#[test]
fn resolve_key_addressed_cells_returns_one_cell_per_keyed_edge() {
    // A COMPOSITE downstream grain, one column literally sourced from each
    // upstream (`a.user_id`, `b.org_id`) — `derive_affected_keys` resolves
    // a `KeyScope` over the model's FULL declared grain for whichever edge
    // at least one grain column depends on
    // (`crates/smelt-logical/src/analysis/affected_keys.rs`'s "sound
    // over-approximation" contract), so both edges declare BOTH grain
    // columns as their own `KeyedUpsert` keys to satisfy the resolver's
    // own key_scope-subset-of-upstream-keys fail-loud check.
    let text = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: [user_id, org_id]\n\
         ---\n\
         SELECT a.user_id, b.org_id, a.total AS a_total, b.total AS b_total\n\
         FROM smelt.models.agg_a a\n\
         JOIN smelt.models.agg_b b ON a.user_id = b.user_id\n";
    let (metadata, sql) = metadata_and_sql(text);
    let edges = vec![
        keyed_edge("agg_a", &["user_id", "org_id"]),
        keyed_edge("agg_b", &["user_id", "org_id"]),
    ];

    let resolved = resolve_live_key_addressed_model_edge_cells(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
    )
    .expect("resolution must not error");

    let mut edge_names: Vec<&str> = resolved.iter().map(|(name, ..)| name.as_str()).collect();
    edge_names.sort_unstable();
    assert_eq!(
        edge_names,
        vec!["agg_a", "agg_b"],
        "expected one resolved cell per keyed upstream edge, got: {edge_names:?}"
    );
}

// ── 3c (phase 4 task 1: fail-loud per cell) ─────────────────────────────────
/// `docs/outcomes/20260816-scheduler-delta-signatures/phases/04-plan.md`
/// test 2: with two keyed edges where one's `key_scope` names a column the
/// upstream relation does not actually carry, the plural resolver still
/// refuses by name — it never silently drops the unhealthy edge and returns
/// only the healthy one.
#[test]
fn resolve_key_addressed_cells_fails_loud_when_one_edge_key_is_missing() {
    // `agg_a` is read straight (its own key column, `user_id`, matches the
    // upstream's declared `KeyedUpsert` key). `agg_b` is read through a
    // renamed alias (`uid`) that the downstream declares as ITS OWN grain
    // column — the row-identity proof resolves a key scope of `uid` for
    // that edge, which does not match `agg_b`'s own declared key column
    // (`user_id`) at all.
    let text = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: uid\n\
         ---\n\
         SELECT a.user_id, b.user_id AS uid, a.total AS a_total, b.total AS b_total\n\
         FROM smelt.models.agg_a a\n\
         JOIN smelt.models.agg_b b ON a.user_id = b.user_id\n";
    let (metadata, sql) = metadata_and_sql(text);
    let edges = vec![
        keyed_edge("agg_a", &["user_id"]),
        keyed_edge("agg_b", &["user_id"]),
    ];

    let result = resolve_live_key_addressed_model_edge_cells(
        &sql,
        "downstream",
        &metadata,
        &[],
        &HashSet::new(),
        &edges,
        SqlDialect::DuckDB,
    );
    // The reachable outcome is either a by-name refusal, or — if the walk
    // cannot resolve a key scope for `agg_a` under this declared grain
    // either — no cells at all. Never a silent partial success (one healthy
    // cell returned, the unhealthy edge dropped).
    match result {
        Err(e) => {
            assert!(
                e.to_string().contains("MaintenanceKeyScopeColumnMissing"),
                "expected a MaintenanceKeyScopeColumnMissing refusal, got: {e}"
            );
        }
        Ok(cells) => {
            assert!(
                cells.is_empty(),
                "a mismatched key-scope column must never resolve alongside a silently \
                 dropped unhealthy edge: {cells:?}"
            );
        }
    }
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

    /// Two clockless `keyed upsert` upstreams (`agg_a`, `agg_b`) both feed
    /// ONE `grain: partition` downstream via a `FULL OUTER JOIN` keyed on
    /// `COALESCE(a.user_id, b.user_id)` — the downstream's own grain column
    /// literally depends on BOTH sides of the join, so BOTH edges resolve a
    /// live key-addressed cell (phase 4 test 3,
    /// `docs/outcomes/20260816-scheduler-delta-signatures/phases/
    /// 04-plan.md`) and the coverage gate can dispatch both in one tick.
    fn stage_two_keyed_upstreams_project(project_dir: &std::path::Path) {
        write(
            project_dir,
            "smelt.yml",
            "name: key_addressed_chain\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    schema: main\n\
             default_materialization: view\n",
        );
        for src in ["payments_a", "payments_b"] {
            write(
                project_dir,
                &format!("models/sources/{src}.yml"),
                "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
                 - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
                 mutation_profile:\n  kind: append_only\n\
                 timeseries:\n  partition_column: d\n  event_time_column: d\n  \
                 granularity: day\n",
            );
        }
        write(
            project_dir,
            "models/agg_a.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments_a\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/agg_b.sql",
            "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
             unique_key: user_id\n---\n\
             SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments_b\n\
             GROUP BY user_id\n",
        );
        write(
            project_dir,
            "models/downstream.sql",
            "---\nmaterialization: table\ntimeseries:\n  event_time_column: d\n  \
             partition_column: d\n  granularity: day\nrefresh: incremental\n\
             grain: partition\n---\n\
             SELECT DATE '2024-01-01' AS d, COALESCE(a.user_id, b.user_id) AS user_id, \
             ANY_VALUE(a.total) AS total_a, ANY_VALUE(b.total) AS total_b \
             FROM smelt.agg_a a FULL OUTER JOIN smelt.agg_b b ON a.user_id = b.user_id \
             GROUP BY COALESCE(a.user_id, b.user_id)\n",
        );
    }

    async fn seed_payments_table(backend: &dyn smelt_backend::Backend, table: &str) {
        backend
            .execute_sql(&format!(
                "CREATE TABLE main.{table} (user_id INTEGER, amount DECIMAL(10,2), d DATE)"
            ))
            .await
            .expect("create payments source table");
        backend
            .execute_sql(&format!(
                "INSERT INTO main.{table} VALUES \
                 (1, 100.00, DATE '2025-01-01'), (2, 70.00, DATE '2025-01-01'), \
                 (3, 30.00, DATE '2025-01-01')"
            ))
            .await
            .expect("seed payments");
    }

    /// A recording [`smelt_runtime::RunReporter`] that captures every
    /// `dispatch_widened` advisory — the visible leg of §"Widen-never-
    /// narrow at dispatch" (phase 4 tasks 6–7).
    #[derive(Default)]
    struct RecordingReporter {
        widened: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl smelt_runtime::RunReporter for RecordingReporter {
        fn dispatch_widened(&self, _run_id: &str, model: &str, reason: &str) {
            self.widened
                .lock()
                .expect("lock")
                .push((model.to_string(), reason.to_string()));
        }
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
            keyed_restrictions: std::collections::BTreeMap::new(),
        }
    }

    /// [`select_request`] plus a populated `keyed_restrictions` map — the
    /// phase 5 tests' own request-scoped keyed-restriction channel
    /// (`ExecuteRequest::keyed_restrictions`,
    /// `docs/specs/incremental_models.md` §"Restrictions compose by union").
    fn select_request_with_restriction(
        models: &[&str],
        consumer: &str,
        upstream: &str,
        keys: &[&str],
        values: &[&str],
    ) -> smelt_runtime::types::ExecuteRequest {
        let mut request = select_request(models);
        request.keyed_restrictions.insert(
            consumer.to_string(),
            vec![smelt_runtime::types::KeyedRestriction {
                upstream: upstream.to_string(),
                keys: keys.iter().map(|s| s.to_string()).collect(),
                values: values.iter().map(|s| s.to_string()).collect(),
            }],
        );
        request
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

    // ── 7 (phase 5: propagated key restrictions reach the cell) ─────────
    /// Phase 5 (`docs/outcomes/20260816-scheduler-delta-signatures/phases/
    /// 05-plan.md` tests 1-2, `docs/specs/incremental_models.md`
    /// §"Restrictions compose by union"): the sidecar diff alone cannot
    /// detect every kind of staleness (here, a row directly corrupted
    /// without any real upstream mutation) — a propagated keyed restriction
    /// naming that key must still reach and repair the cell, even though
    /// the sidecar itself reports zero changed keys (proving dispatch
    /// actually runs rather than short-circuiting to the `Ok(None)` no-op).
    #[tokio::test]
    async fn propagated_restriction_key_is_repaired_when_sidecar_reports_no_change() {
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

        // Run 1: creation — nothing to repair yet.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "restrict-run-1".to_string(),
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

        // Mutate user 1's contribution so run 2 dispatches a REAL repair —
        // this establishes the group-grain sidecar's own baseline for BOTH
        // keys (the refresh self-heals every currently-observed key, not
        // just the changed subset).
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
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "restrict-run-2".to_string(),
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
            .expect("second run (establish sidecar baseline) must succeed");
        }

        // Directly corrupt user 2's downstream row WITHOUT touching
        // payments/agg at all — the sidecar's own digest for user 2 is
        // still accurate (nothing upstream changed), so a plain repair
        // dispatch finds zero changed keys.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.downstream SET total = 999.00 WHERE user_id = 2")
                .await
                .expect("corrupt downstream row");
        }

        // Run 3 (no restriction): the sidecar alone cannot see the
        // corruption — the corrupted value survives untouched.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "restrict-run-3".to_string(),
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
            .expect("third run (no restriction, sidecar-only) must succeed");
        }
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            let still_corrupted = scalar_text(
                &backend,
                "SELECT total FROM main.downstream WHERE user_id = 2",
            )
            .await;
            assert_eq!(
                still_corrupted, "999.00",
                "the sidecar diff alone must not detect the direct corruption — this is the \
                 baseline the propagated restriction must fix"
            );
        }

        // Run 4: a propagated keyed restriction naming user 2 on the `agg`
        // edge — the sidecar's own diff is still empty, but the restriction
        // alone must dispatch the cell and repair the row.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "restrict-run-4".to_string(),
                select_request_with_restriction(
                    &["downstream"],
                    "downstream",
                    "agg",
                    &["user_id"],
                    &["2"],
                ),
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
            .expect("fourth run (propagated restriction) must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_eq!(record.strategy, "per_group_recompute");
        }

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let repaired = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 2",
        )
        .await;
        assert_eq!(
            repaired, "70.00",
            "the propagated restriction alone must dispatch the cell and repair user 2's group \
             back to the true upstream value, even though the sidecar diff reported no change"
        );
    }

    // ── 9 (dispatch outside `grain: key`) ──────────────────────────────
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

    // ── 10 (substitution gate: an uncovered second input) ──────────────
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

    // ── 11 (dispatch composition: two covered inbound edges) ───────────
    /// Phase 4 test 3
    /// (`docs/outcomes/20260816-scheduler-delta-signatures/phases/
    /// 04-plan.md`): a `grain: partition` downstream reading TWO clockless
    /// `keyed upsert` upstreams — one key changes in EACH upstream in the
    /// SAME tick — dispatches BOTH resolved key-addressed cells rather than
    /// falling back to the ordinary route as soon as a second covered edge
    /// appears (phase 2's single-edge gate). The result equals a
    /// full-refresh oracle, and a key touched by neither upstream (user 3)
    /// is bit-identical.
    #[tokio::test]
    async fn two_keyed_upstreams_dispatch_both_cells() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_two_keyed_upstreams_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments_table(&backend, "sources_payments_a").await;
            seed_payments_table(&backend, "sources_payments_b").await;
        }

        let models = ["agg_a", "agg_b", "downstream"];

        // Run 1: creation.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "two-keyed-run-1".to_string(),
                select_request(&models),
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

        // Mutate user 1 in `payments_a` and user 2 in `payments_b` — a
        // DIFFERENT key changes in EACH upstream. User 3 is untouched in
        // BOTH.
        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            use smelt_backend::Backend;
            backend
                .execute_sql(
                    "UPDATE main.sources_payments_a SET amount = 200.00 \
                     WHERE user_id = 1 AND amount = 100.00",
                )
                .await
                .expect("mutate payments_a");
            backend
                .execute_sql(
                    "UPDATE main.sources_payments_b SET amount = 300.00 \
                     WHERE user_id = 2 AND amount = 70.00",
                )
                .await
                .expect("mutate payments_b");
        }

        // Run 2: `downstream` must resolve and dispatch BOTH key-addressed
        // cells in one tick.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "two-keyed-run-2".to_string(),
                select_request(&models),
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
            .expect("second run (composed key-addressed recompute) must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_eq!(
                record.strategy, "per_group_recompute",
                "a downstream with two fully-covered key-addressed inbound edges must dispatch \
                 the repair family's composed cells, not the ordinary route"
            );
        }

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");

        let user1 = scalar_text(
            &backend,
            "SELECT total_a || ',' || total_b FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(
            user1, "200.00,100.00",
            "user 1's total_a must reflect agg_a's mutated contribution; total_b (from the \
             untouched agg_b) must be unchanged"
        );
        let user2 = scalar_text(
            &backend,
            "SELECT total_a || ',' || total_b FROM main.downstream WHERE user_id = 2",
        )
        .await;
        assert_eq!(
            user2, "70.00,300.00",
            "user 2's total_b must reflect agg_b's mutated contribution; total_a (from the \
             untouched agg_a) must be unchanged"
        );
        let user3 = scalar_text(
            &backend,
            "SELECT total_a || ',' || total_b FROM main.downstream WHERE user_id = 3",
        )
        .await;
        assert_eq!(
            user3, "30.00,30.00",
            "user 3's group must be bit-identical — it was never in either upstream's affected-\
             key set"
        );

        // Cross-check against a full-refresh oracle over the CURRENT source
        // state.
        let oracle_a1 = scalar_text(
            &backend,
            "SELECT SUM(amount) FROM main.sources_payments_a WHERE user_id = 1",
        )
        .await;
        let oracle_b2 = scalar_text(
            &backend,
            "SELECT SUM(amount) FROM main.sources_payments_b WHERE user_id = 2",
        )
        .await;
        assert_eq!(oracle_a1, "200.00");
        assert_eq!(oracle_b2, "300.00");
    }

    // ── 12 (widen-never-narrow: the visible downgrade) ─────────────────
    /// Phase 4 test 4: the SAME uncovered-input fixture test 8 exercises,
    /// now run with a [`RecordingReporter`] — the ordinary route is still
    /// taken and the result is still correct, but exactly one
    /// `dispatch_widened` advisory fires naming the uncovered input
    /// (§"Widen-never-narrow at dispatch": "an explain-visible downgrade,
    /// never … silently").
    #[tokio::test]
    async fn uncovered_input_widens_and_reports_the_downgrade() {
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

        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "widen-run-1".to_string(),
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
            use smelt_backend::Backend;
            backend
                .execute_sql("UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND amount = 100.00")
                .await
                .expect("mutate payments");
        }

        let reporter = RecordingReporter::default();
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = smelt_runtime::execute_project(
                "widen-run-2".to_string(),
                select_request(&["agg", "downstream"]),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &reporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second run must succeed");
            let record = outcome.models.get("downstream").expect("downstream ran");
            assert_ne!(
                record.strategy, "per_group_recompute",
                "the ordinary route must still run when an input is uncovered"
            );
        }

        let widened = reporter.widened.lock().expect("lock").clone();
        assert_eq!(
            widened.len(),
            1,
            "expected exactly one dispatch_widened advisory, got: {widened:?}"
        );
        assert_eq!(widened[0].0, "downstream");
        assert!(
            widened[0].1.contains("flags"),
            "the advisory must name the uncovered input, got: {}",
            widened[0].1
        );

        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let repaired = scalar_text(
            &backend,
            "SELECT total FROM main.downstream WHERE user_id = 1",
        )
        .await;
        assert_eq!(repaired, "250.00");
    }

    // ── 13 (widen-never-narrow: no downgrade when fully covered) ───────
    /// Phase 4 test 5: the fully-covered two-upstream fixture (test 9)
    /// fires no `dispatch_widened` advisory — the downgrade report is only
    /// for the genuinely uncovered case.
    #[tokio::test]
    async fn full_coverage_reports_no_downgrade() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("run.duckdb");
        stage_two_keyed_upstreams_project(&project_dir);
        let config =
            Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

        {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            seed_payments_table(&backend, "sources_payments_a").await;
            seed_payments_table(&backend, "sources_payments_b").await;
        }

        let models = ["agg_a", "agg_b", "downstream"];

        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "full-coverage-run-1".to_string(),
                select_request(&models),
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
            use smelt_backend::Backend;
            backend
                .execute_sql(
                    "UPDATE main.sources_payments_a SET amount = 200.00 \
                     WHERE user_id = 1 AND amount = 100.00",
                )
                .await
                .expect("mutate payments_a");
        }

        let reporter = RecordingReporter::default();
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            smelt_runtime::execute_project(
                "full-coverage-run-2".to_string(),
                select_request(&models),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &DuckDbBackendFactory {
                    db_path: db_path.clone(),
                },
                &reporter,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second run must succeed");
        }

        let widened = reporter.widened.lock().expect("lock");
        assert!(
            widened.is_empty(),
            "a fully-covered downstream must fire no dispatch_widened advisory, got: {widened:?}"
        );
    }
}
