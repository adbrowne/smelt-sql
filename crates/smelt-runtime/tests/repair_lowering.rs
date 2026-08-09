//! Runtime lowering for the repair family (`docs/specs/incremental_models.md`
//! §"The repair family"): an admitted `Technique::PerGroupRecompute` cell
//! resolves live off the derived maintenance plan, its two emitter string
//! inputs are built by pure builders, and the resulting `StatementGroup`
//! repairs exactly the affected key groups.
//!
//! The unit legs (1–4) need no backend; the DuckDB legs (5–6) drive a real
//! `execute_project` run so the repair is proven against stored state, not
//! against a hand-assembled statement list.

use std::collections::HashSet;

use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::emit::Region;
use smelt_logical::maintenance::{MutationProfile, RowIdentity, ScanClamp, SourceFacts, Technique};
use smelt_runtime::maintenance_driver::{
    repair_affected_keys_select, repair_candidate_select, resolve_live_per_group_recompute_cell,
};

/// The keyed fixture this file's unit legs share: `MAX` (a non-invertible
/// combiner under retraction) folded per `customer_id` over `raw.orders`, a
/// **clocked** `mutable_snapshot` source. The explicit Form B band on
/// `order_date` is what discharges the repair family's obligation 4
/// (bounded per-group read footprint) — without it
/// `repair::admit_per_group_recompute` refuses `RepairSliceUnbounded`.
const REPAIR_MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
     '2025-01-12' \
     GROUP BY customer_id";

const REPAIR_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     ---\n";

/// An ordinary append-only keyed fold — the shape the repair narrowing must
/// leave alone (the faithful-fold source-posture obligation passes, so a
/// `Technique::KeyedFold` cell is admitted and no repair cell exists).
const APPEND_ONLY_MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
     '2025-01-12' \
     GROUP BY customer_id";

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

fn orders_source(mutation: MutationProfile) -> SourceFacts {
    SourceFacts {
        name: "raw.orders".to_string(),
        mutation,
        partition_col: Some("order_date".to_string()),
        unique_key: vec!["order_id".to_string()],
        allow_full_scan: false,
    }
}

fn mutable_orders() -> (Vec<SourceFacts>, HashSet<String>) {
    let mut explicitly_mutable = HashSet::new();
    explicitly_mutable.insert("raw.orders".to_string());
    (
        vec![orders_source(MutationProfile::MutableSnapshot)],
        explicitly_mutable,
    )
}

// ── 1 ────────────────────────────────────────────────────────────────────
#[test]
fn resolve_live_per_group_recompute_cell_finds_the_admitted_repair_cell() {
    let text = format!("{REPAIR_MODEL_FILE}{REPAIR_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let (sources, explicitly_mutable) = mutable_orders();

    let (source, cell, key, slice) = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
    )
    .expect("resolver must not error")
    .expect("a live per-group-recompute cell must resolve for raw.orders");

    assert_eq!(source, "raw.orders");
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    // The key is the cell's own `RowIdentity::Key`, verbatim — never
    // re-derived from the model's declared `unique_key`.
    assert_eq!(
        cell.row_identity.identity,
        RowIdentity::Key(key.clone()),
        "the returned key must be the cell's own proven row identity"
    );
    assert_eq!(key, vec!["customer_id".to_string()]);
    assert_eq!(slice.source, "raw.orders");
    assert_eq!(slice.column, "order_date");
    assert_eq!(slice.before, Seconds::days(1));
}

// ── 2 ────────────────────────────────────────────────────────────────────
#[test]
fn resolve_live_per_group_recompute_cell_none_for_an_append_only_model() {
    let text = format!("{REPAIR_MODEL_FILE}{APPEND_ONLY_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let sources = vec![orders_source(MutationProfile::AppendOnly)];

    let resolved = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &HashSet::new(),
        &[],
    )
    .expect("resolver must not error");

    assert!(
        resolved.is_none(),
        "repair never displaces an admitted fold cell, got {resolved:?}"
    );
}

// ── 3 ────────────────────────────────────────────────────────────────────
/// A `PerGroupRecompute` cell carrying `RowIdentity::WholeRow` is
/// structurally impossible today (`repair::derive_repair_cell` always builds
/// `RowIdentity::Key`), so this exercises the resolver's fail-loud leg
/// directly on a synthetic cell: it must error by name rather than silently
/// widening the repair to every stored row.
#[test]
fn resolve_live_per_group_recompute_cell_fails_loud_on_whole_row_identity() {
    use smelt_logical::maintenance::{
        Corner, PartitionLocal, PlanCell, RowIdentityVerdict, Trigger,
    };

    let cell = PlanCell {
        group: "{max_amount}".to_string(),
        trigger: Trigger::NewData {
            source: "raw.orders".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::PerGroupRecompute,
        partition_local: PartitionLocal::Yes,
        scans: vec![ScanClamp {
            source: "raw.orders".to_string(),
            column: "order_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        }],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
    };

    let err = smelt_runtime::maintenance_driver::repair_cell_key(&cell)
        .expect_err("a WholeRow per-group-recompute cell must fail loud");
    let msg = err.to_string();
    assert!(
        msg.contains("RowIdentity::WholeRow") && msg.contains("PerGroupRecompute"),
        "the refusal must name the failing identity and technique, got: {msg}"
    );
}

// ── 4 ────────────────────────────────────────────────────────────────────
#[test]
fn affected_keys_select_bounds_the_read_with_the_cells_scan_clamp() {
    let key = vec!["customer_id".to_string()];
    let region = Region {
        start: "'2025-01-12'".to_string(),
        end: "'2025-01-13'".to_string(),
    };
    let clamp = ScanClamp {
        source: "raw.orders".to_string(),
        column: "order_date".to_string(),
        before: Seconds::days(1),
        after: Seconds::ZERO,
    };

    let clamped =
        repair_affected_keys_select("main.sources_raw_orders", &key, Some(&clamp), &region);
    assert_eq!(
        clamped,
        "SELECT DISTINCT customer_id FROM main.sources_raw_orders WHERE order_date >= \
         '2025-01-12' - INTERVAL '86400 seconds' AND order_date < '2025-01-13'",
        "the affected-keys read must carry the cell's own derived clamp predicate"
    );

    // Only reachable where admission already proved the slice by another
    // route — an unclamped scan is the unpredicated read, never a
    // silently-narrowed one.
    let unclamped = repair_affected_keys_select("main.sources_raw_orders", &key, None, &region);
    assert_eq!(
        unclamped,
        "SELECT DISTINCT customer_id FROM main.sources_raw_orders"
    );

    // The candidate carries NO clamp of its own — the group is recomputed
    // whole, semi-joined to the affected keys.
    let candidate = repair_candidate_select("SELECT customer_id, 1 AS v", &key, &clamped);
    assert!(
        !candidate.contains("INTERVAL '86400 seconds' AND order_date <")
            || candidate.matches("86400 seconds").count() == 1,
        "the clamp belongs to the affected-keys read only, not the output wrapper: {candidate}"
    );
    assert!(
        candidate.contains("EXISTS") && candidate.contains("SELECT customer_id, 1 AS v"),
        "the candidate is the model's own full SQL semi-joined to the affected keys: {candidate}"
    );
}

// ── DuckDB-backed legs (5, 6) ────────────────────────────────────────────
//
// A real `execute_project` run over a staged project: creation folds (there
// is nothing to repair yet), then a retraction in one group is repaired by
// the per-group recompute — never by the fold, whose combiner (`MAX`) cannot
// undo a retracted contribution.

/// The mutable, clocked source the repair fixture folds over. Clocked
/// (`timeseries:`) so `derive_model_maintenance_plan` derives no
/// `UpstreamMutation` trigger for it — its post-creation mutations are the
/// repair cell's job on the model's own `NewData` trigger, not an
/// enrichment cell's.
pub const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;

/// The keyed repair fixture: `MAX` per `customer_id` over the mutable
/// source, with an explicit 3-day Form B band on `order_date` — the band
/// discharges the repair family's bounded-per-group-read obligation and
/// fixes the derived `ScanClamp` at `before = 3 days`.
pub const REPAIR_FIXTURE_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
     '2025-01-14' \
     GROUP BY customer_id";

pub const REPAIR_FIXTURE_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     ---\n";

/// The full-refresh oracle for [`REPAIR_FIXTURE_SQL`], against the physical
/// source table.
pub const REPAIR_FIXTURE_ORACLE: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM main.sources_raw_orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
     '2025-01-14' \
     GROUP BY customer_id";

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

/// Stage `examples/timeseries` into `project_dir`, adding the mutable
/// `raw.orders` source and the keyed repair model.
pub fn stage_repair_project(project_dir: &std::path::Path) {
    let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    copy_dir_recursive(&source_dir, project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{REPAIR_FIXTURE_FILE}{REPAIR_FIXTURE_SQL}\n"),
    )
    .expect("write repair model fixture");
}

/// Seed the physical source table. Customer 1's group has two rows inside
/// the widened affected-key window (`order_date >= run_start − 3 days`);
/// customer 2's single row sits at `2025-01-11`, inside the model's own band
/// but OUTSIDE run 2's affected-key window — so it is provably untouched by
/// the repair's targeted `DELETE`/`INSERT`.
pub async fn seed_orders(backend: &dyn smelt_backend::Backend) {
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
             amount DECIMAL(10,2), order_date TIMESTAMP)",
        )
        .await
        .expect("create orders source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_orders VALUES \
             (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
             (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
             (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
        )
        .await
        .expect("seed orders");
}

pub fn build_db_and_graph(
    project_dir: &std::path::Path,
    config: &smelt_core::config::Config,
) -> (
    std::sync::Arc<tokio::sync::Mutex<smelt_db::Database>>,
    std::sync::Arc<tokio::sync::Mutex<smelt_core::graph::DependencyGraph>>,
) {
    use smelt_core::ModelDiscovery;
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(
        config
            .target
            .clone()
            .map(|t| std::sync::Arc::from(t.as_str())),
    );

    let graph = smelt_core::graph::DependencyGraph::build(sql_models, None).expect("build graph");

    (
        std::sync::Arc::new(tokio::sync::Mutex::new(db)),
        std::sync::Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

pub fn select_request(
    target: &str,
    model: &str,
    start: &str,
    end: &str,
) -> smelt_runtime::types::ExecuteRequest {
    smelt_runtime::types::ExecuteRequest {
        target: target.to_string(),
        select: vec![model.to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
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

pub struct DuckDbBackendFactory {
    pub db_path: std::path::PathBuf,
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

/// Scalar `SELECT` returning one DECIMAL rendered as text, for
/// bit-identity assertions that do not depend on Arrow decimal decoding.
pub async fn scalar_text(backend: &dyn smelt_backend::Backend, sql: &str) -> String {
    let batches = backend.execute_sql(sql).await.expect("query");
    let batch = batches.first().expect("one batch");
    assert_eq!(batch.num_rows(), 1, "expected exactly one row for: {sql}");
    let col = batch.column(0);
    arrow::util::display::array_value_to_string(col, 0).expect("render value")
}

async fn run_repair_fixture(
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
) -> smelt_runtime::types::RunOutcome {
    use std::sync::Arc;
    stage_repair_project(project_dir);
    let config = Arc::new(smelt_core::config::Config::load(project_dir).expect("load smelt.yml"));

    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
            .await
            .expect("open duckdb");
        seed_orders(&backend).await;
    }

    // Run 1: creation. There is nothing to repair yet, so the fold's own
    // create path materializes the table.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        smelt_runtime::execute_project(
            "repair-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &DuckDbBackendFactory {
                db_path: db_path.to_path_buf(),
            },
            &smelt_runtime::NoOpReporter,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // The retraction: customer 1's top-of-group contribution is corrected
    // downward in place. `MAX` cannot undo it — a fold would leave 100.00.
    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
            .await
            .expect("reopen duckdb");
        use smelt_backend::Backend;
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    // Run 2: the affected-key window is `order_date >= 2025-01-16 − 3 days`
    // (= 2025-01-13), so customer 1's group is affected and customer 2's
    // (2025-01-11) is not.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    smelt_runtime::execute_project(
        "repair-run-2".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("second run (per-group recompute) must succeed")
}

// ── 5 ────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn per_group_recompute_repairs_only_the_affected_group() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    drop(backend);

    let outcome = run_repair_fixture(&project_dir, &db_path).await;
    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the retraction must dispatch the repair family, not the fold"
    );

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let repaired = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 1",
    )
    .await;
    assert_eq!(
        repaired, "50.00",
        "the affected group must be recomputed whole — a fold would have left the retracted \
         100.00 in place"
    );
    let untouched = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 2",
    )
    .await;
    assert_eq!(
        untouched, "70.00",
        "a group outside the affected-key window must be bit-identical after the repair"
    );
}

// ── 6 ────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn per_group_recompute_matches_full_refresh_after_retraction() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    drop(backend);

    run_repair_fixture(&project_dir, &db_path).await;

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    assert!(
        multiset_equal(
            &backend,
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            REPAIR_FIXTURE_ORACLE,
        )
        .await,
        "the incremental state after the repair must equal a full refresh over the same inputs"
    );
}

/// `EXCEPT ALL` both ways — the Link-C multiset oracle this workspace's
/// other equivalence legs use (`statement_parity.rs::multiset_equal`).
pub async fn multiset_equal(
    backend: &dyn smelt_backend::Backend,
    left_sql: &str,
    right_sql: &str,
) -> bool {
    except_all_count(backend, left_sql, right_sql).await == 0
        && except_all_count(backend, right_sql, left_sql).await == 0
}

async fn except_all_count(
    backend: &dyn smelt_backend::Backend,
    left_sql: &str,
    right_sql: &str,
) -> i64 {
    use arrow::array::{Array, Int64Array};
    let sql = format!("SELECT COUNT(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d");
    let batches = backend.execute_sql(&sql).await.expect("except all query");
    let batch = batches.first().expect("one batch");
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count column");
    col.value(0)
}
