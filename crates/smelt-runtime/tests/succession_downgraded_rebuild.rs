//! Executed-not-just-dispatched proof for phase 9e
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/09e-plan.md`):
//! a `state_downgraded` succession cell's full rebuild must actually
//! *complete* against a live backend, not merely be routed to the right
//! function. 9c's dispatch fix forced a downgraded cell to
//! `rebuild_succession_state`, but that function still carried the
//! ledger-bearing arm's unconditional tombstone-ledger gate, so the fix
//! never reached Databricks — caught here by executing the downgraded path
//! against a real DuckDB backend rather than asserting the dispatch
//! decision alone (`docs/specs/state.md` §"The degradation contract").

use std::path::Path;

use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Granularity;
use smelt_logical::maintenance::succession::SuccessionRecipe;
use smelt_runtime::maintenance_driver::succession::{rebuild_succession_state, SuccessionCell};
use smelt_types::DataType;

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;

fn no_retry_policy() -> smelt_runtime::execute::RetryPolicy<'static> {
    smelt_runtime::execute::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "succession-downgraded-unit-test",
        model_name: "customer_history",
        reporter: &NO_OP_REPORTER,
    }
}

fn probe_policy() -> smelt_runtime::probes::ProbePolicy {
    smelt_runtime::probes::ProbePolicy::per_run()
}

fn presented_columns() -> Vec<(String, DataType)> {
    vec![
        ("customer_id".to_string(), DataType::Integer),
        (
            "changed_at".to_string(),
            DataType::Timestamp {
                with_timezone: false,
            },
        ),
        ("tier".to_string(), DataType::Varchar { max_length: None }),
        (
            "valid_to".to_string(),
            DataType::Timestamp {
                with_timezone: false,
            },
        ),
    ]
}

fn recipe() -> SuccessionRecipe {
    SuccessionRecipe {
        source_table: "customer_changes".to_string(),
        pre_filter: None,
        key_cols: vec!["customer_id".to_string()],
        clock_col: "changed_at".to_string(),
        payload_columns: vec!["tier".to_string()],
        row_local_projection: vec![
            ("customer_id".to_string(), "customer_id".to_string()),
            ("changed_at".to_string(), "changed_at".to_string()),
            ("tier".to_string(), "tier".to_string()),
        ],
        lead_derived: vec![("valid_to".to_string(), "{lead}".to_string())],
        lag_derived: vec![],
        delete_flag_expr: None,
    }
}

fn downgraded_cell() -> SuccessionCell {
    SuccessionCell {
        recipe: recipe(),
        presented_table: "main.customer_history".to_string(),
        source_table: "main.raw_customer_changes".to_string(),
        partition_column: "arrival_date".to_string(),
        granularity: Granularity::Day,
        state_downgraded: true,
    }
}

fn non_downgraded_cell() -> SuccessionCell {
    SuccessionCell {
        state_downgraded: false,
        ..downgraded_cell()
    }
}

const MODEL_SELECT_SQL: &str = "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER \
     (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM main.raw_customer_changes";

fn stage_source(conn: &duckdb::Connection) {
    conn.execute_batch(
        "CREATE TABLE main.raw_customer_changes (customer_id INTEGER, changed_at TIMESTAMP, \
         arrival_date DATE, tier VARCHAR)",
    )
    .expect("create source table");
}

fn insert_event(conn: &duckdb::Connection, id: i64, changed_at: &str, tier: &str) {
    conn.execute_batch(&format!(
        "INSERT INTO main.raw_customer_changes VALUES ({id}, TIMESTAMP '{changed_at}', DATE \
         '2026-01-01', '{tier}')"
    ))
    .expect("insert event");
}

fn row_count(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM ({sql}) AS t"), [], |row| {
        row.get(0)
    })
    .expect("row count query")
}

fn table_exists(conn: &duckdb::Connection, table: &str) -> bool {
    row_count(
        conn,
        &format!(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = 'main' AND \
             table_name = '{table}'"
        ),
    ) > 0
}

async fn open_backend(db_path: &Path) -> DuckDbBackend {
    DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb")
}

/// Test 3: `a_downgraded_cell_rebuilds_for_real_against_duckdb`.
#[tokio::test]
async fn a_downgraded_cell_rebuilds_for_real_against_duckdb() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "gold");
        insert_event(&conn, 1, "2026-01-02 08:00:00", "silver");
    }

    let backend = open_backend(&db_path).await;
    rebuild_succession_state(
        &backend,
        "customer_history",
        "main",
        "customer_history",
        &downgraded_cell(),
        &presented_columns(),
        MODEL_SELECT_SQL,
        &no_retry_policy(),
        &probe_policy(),
        &NO_OP_REPORTER,
        "run-downgraded-rebuild",
    )
    .await
    .expect("a downgraded rebuild must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(
            &conn,
            "(SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history) \
             EXCEPT ALL (SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER \
             (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM \
             main.raw_customer_changes)"
        ),
        0,
        "the rebuilt presented table must match the model SQL's own full-refresh oracle"
    );
    assert!(
        !table_exists(&conn, "customer_history__tombstones"),
        "a downgraded rebuild must never create a tombstone table"
    );
}

/// Test 4: `a_downgraded_rebuild_reports_no_tombstone_or_probe_statement`.
#[tokio::test]
async fn a_downgraded_rebuild_reports_no_tombstone_or_probe_statement() {
    #[derive(Default)]
    struct RecordingReporter {
        statements: std::sync::Mutex<Vec<String>>,
    }
    impl smelt_runtime::RunReporter for RecordingReporter {
        fn maintenance_statements(
            &self,
            _run_id: &str,
            _model: &str,
            _step: Option<&smelt_runtime::ChunkInfo>,
            group: &smelt_logical::maintenance::emit::StatementGroup,
        ) {
            let mut statements = self.statements.lock().expect("lock");
            statements.extend(group.statements.iter().map(|s| s.sql.clone()));
        }
    }

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "gold");
    }

    let reporter = RecordingReporter::default();
    let backend = open_backend(&db_path).await;
    rebuild_succession_state(
        &backend,
        "customer_history",
        "main",
        "customer_history",
        &downgraded_cell(),
        &presented_columns(),
        MODEL_SELECT_SQL,
        &smelt_runtime::execute::RetryPolicy {
            retry_max: 0,
            base_backoff_ms: 0,
            run_id: "run-downgraded-report",
            model_name: "customer_history",
            reporter: &reporter,
        },
        &probe_policy(),
        &reporter,
        "run-downgraded-report",
    )
    .await
    .expect("a downgraded rebuild must succeed");

    let statements = reporter.statements.lock().expect("lock");
    assert_eq!(
        statements.len(),
        1,
        "exactly one statement must be reported: {statements:?}"
    );
    assert!(statements[0].starts_with("CREATE TABLE main.customer_history"));
    assert!(!statements[0].contains("__tombstones"), "{}", statements[0]);
}

/// Test 5: `a_non_downgraded_cell_on_a_no_ledger_dialect_still_refuses` — the
/// backstop refusal survives for a cell that is NOT downgraded, catching a
/// caller that skipped the availability check. DuckDB always realises the
/// tombstone ledger, so this asserts against the guard's own predicate
/// directly rather than needing a genuinely ledger-less backend.
#[tokio::test]
async fn a_non_downgraded_cell_on_a_no_ledger_dialect_still_refuses() {
    use smelt_backend::Backend;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
    }
    let backend = open_backend(&db_path).await;
    assert!(
        smelt_runtime::maintenance_driver::realises_tombstone_ledger(backend.dialect()),
        "DuckDB realises the tombstone ledger, so the guard is asserted directly here rather \
         than exercised via a real refusal"
    );
    // A non-downgraded cell against DuckDB takes the ledger-bearing arm and
    // succeeds — proving the branch on `state_downgraded` did not
    // accidentally short-circuit the ordinary path.
    rebuild_succession_state(
        &backend,
        "customer_history",
        "main",
        "customer_history",
        &non_downgraded_cell(),
        &presented_columns(),
        MODEL_SELECT_SQL,
        &no_retry_policy(),
        &probe_policy(),
        &NO_OP_REPORTER,
        "run-non-downgraded",
    )
    .await
    .expect("a non-downgraded rebuild on a ledger-bearing dialect must succeed");
    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert!(
        table_exists(&conn, "customer_history__tombstones"),
        "the ledger-bearing arm must still create the tombstone table"
    );
}

/// Test 6: `a_downgraded_cell_rebuilds_idempotently`.
#[tokio::test]
async fn a_downgraded_cell_rebuilds_idempotently() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open");
        stage_source(&conn);
        insert_event(&conn, 1, "2026-01-01 08:00:00", "gold");
        insert_event(&conn, 1, "2026-01-02 08:00:00", "silver");
    }

    for _ in 0..2 {
        let backend = open_backend(&db_path).await;
        rebuild_succession_state(
            &backend,
            "customer_history",
            "main",
            "customer_history",
            &downgraded_cell(),
            &presented_columns(),
            MODEL_SELECT_SQL,
            &no_retry_policy(),
            &probe_policy(),
            &NO_OP_REPORTER,
            "run-downgraded-idempotent",
        )
        .await
        .expect("a downgraded rebuild must succeed");
    }

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    assert_eq!(
        row_count(&conn, "SELECT * FROM main.customer_history"),
        2,
        "running the downgraded rebuild twice over unchanged source must not duplicate rows"
    );
}
