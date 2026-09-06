//! Real-fixture, DuckDB-backed coverage for BL7's window-independence /
//! ordered-execution composition (`docs/specs/incremental_models.md`
//! §"Window independence and self-referential models"), exercised through
//! the genuine `execute_project` path — the single shared entry point both
//! `smelt-cli` and `smelt-ui` call (root `CLAUDE.md` §Architectural
//! invariants, "Run pipeline parity rule").
//!
//! Fixture: `marts.running_balance` reads its own prior day's balance via a
//! self-join (`smelt.marts.running_balance` — `smelt.<self>` in spec prose)
//! bounded exactly one day back, matching F10's convergent (`Ordered`) case
//! (`crates/smelt-logical/src/analysis/window_independence.rs`'s own
//! `backward_bounded_self_edge_is_ordered` unit test uses the identical SQL
//! shape). A 3-day backfill run in one `execute_project` call must build the
//! partitions strictly in temporal order, each reading the prior day's
//! already-committed balance, converging to the same running total a
//! hand-computed sequential build would produce.
//!
//! **Bootstrap note.** A self-referential model's very first partition
//! cannot be created via `CREATE TABLE ... AS SELECT ...` (DuckDB refuses to
//! resolve a table reference to itself mid-creation — confirmed directly
//! against the DuckDB CLI while building this fixture). This is a separate,
//! pre-existing gap from BL7's ordering/refusal scope (not in this phase's
//! spec anchor or TDD list) — tracked as deferred in
//! `docs/plans/20260704-model-updates-l4-batched.md` §"Deferred during
//! implementation". This test sidesteps it exactly as
//! `crates/smelt-cli/tests/e2e/two_layer_clamp.rs` sidesteps the analogous
//! source-side lookback gap: seed the target table with one opening-balance
//! row *before* the requested run window, so every batch in the window
//! (including its first) goes through the ordinary DELETE+INSERT path, never
//! `CREATE TABLE AS`.
use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::execute_project;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

/// Stage a project with:
/// - `models/sources/transactions.yml`: a plain (no `timeseries:` needed —
///   this fixture derives no pushdown bound against it) source declaration.
/// - `models/marts/running_balance.sql`: a `refresh: batched` model whose
///   self-join reads its own prior day's row, offset by exactly
///   `INTERVAL '1 day'` — the same Form-A bound shape
///   `window_independence`'s own `backward_bounded_self_edge_is_ordered`
///   unit test proves `Ordered`.
fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::create_dir_all(project_dir.join("models/marts")).unwrap();

    let source_yml = r#"description: Raw per-account transactions.
columns:
  - name: d
    type: DATE
  - name: acct_id
    type: INTEGER
  - name: amount
    type: DOUBLE
"#;
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        source_yml,
    )
    .unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
WITH joined AS (
    SELECT
        t.d AS d,
        t.acct_id AS acct_id,
        bal.balance + t.amount AS balance
    FROM smelt.marts.running_balance bal
    JOIN smelt.sources.transactions t ON bal.acct_id = t.acct_id
    WHERE bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
)
SELECT d, acct_id, balance FROM joined
"#;
    std::fs::write(
        project_dir.join("models/marts/running_balance.sql"),
        model_sql,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: self_referential_ordered_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed the source table (one transaction per day, 2024-01-01..2024-01-03)
/// and the model's own output table with a single opening-balance row dated
/// one day before the run window (2023-12-31) — see the bootstrap note at
/// the top of this file.
fn seed_tables(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_transactions AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 10.0),
            (DATE '2024-01-02', 1, 20.0),
            (DATE '2024-01-03', 1, 30.0)
        ) AS t(d, acct_id, amount);

        CREATE OR REPLACE TABLE main.marts_running_balance AS
        SELECT * FROM (VALUES
            (DATE '2023-12-31', 1, 1000.0)
        ) AS t(d, acct_id, balance);
        "#,
    )?;
    Ok(())
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn fetch_balance(db_path: &Path, d: &str) -> anyhow::Result<f64> {
    let conn = duckdb::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT balance FROM main.marts_running_balance WHERE d = ?::DATE AND acct_id = 1",
    )?;
    let mut rows = stmt.query([d])?;
    if let Some(row) = rows.next()? {
        let val: f64 = row.get(0)?;
        Ok(val)
    } else {
        panic!("expected a running_balance row for d = {d}, found none");
    }
}

/// A 3-day backfill run (one `execute_project` call, `batch_size_days: 1`
/// forcing an explicit per-batch split — though BL7's `Ordered` verdict
/// would force this granularity anyway even without the override) converges
/// to the same running total a hand-computed sequential build produces:
/// 1000 → 1010 → 1030 → 1060. If the self-referential ordering/granularity
/// composition regressed (e.g. batches ran out of temporal order, or were
/// lumped into one wide query reading not-yet-written rows), the later
/// days' balances would be wrong or the rows would be missing entirely.
#[tokio::test]
async fn self_referential_backfill_converges_to_sequential_running_balance() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_tables(&db_path).expect("seed tables");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-04".to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
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
    };

    let outcome = execute_project(
        "self-ref-ordered-backfill".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("a convergent self-edge must build for real, not refuse");

    assert!(
        !outcome.models.is_empty(),
        "expected the running_balance model in the outcome"
    );

    assert_eq!(
        fetch_balance(&db_path, "2024-01-01").expect("day 1 balance"),
        1010.0,
        "day 1: opening 1000 + 10"
    );
    assert_eq!(
        fetch_balance(&db_path, "2024-01-02").expect("day 2 balance"),
        1030.0,
        "day 2: 1010 + 20 — must read day 1's committed row, not the seed row"
    );
    assert_eq!(
        fetch_balance(&db_path, "2024-01-03").expect("day 3 balance"),
        1060.0,
        "day 3: 1030 + 30 — proves strictly sequential temporal-order execution"
    );

    // Exactly one row per day was written — a wide/lumped batch would either
    // produce zero rows (self-join sees no intermediate days at all) or the
    // wrong balances (all days joining against only the seed row).
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main.marts_running_balance WHERE d >= '2024-01-01'::DATE",
            [],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count, 3, "expected exactly 3 rows, one per backfilled day");
}
