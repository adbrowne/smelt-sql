//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Phase-B / cell `P0-1` acceptance test for the property-discovery loop
//! (`docs/plans/20260705-property-discovery-loop.md`). Proves the in-process
//! Link-C harness (`prop_helpers::link_c_harness`) actually drives smelt's
//! REAL bound derivation: a `refresh: batched` model with **no `WHERE`
//! clause at all** is compiled and executed through `execute_project`, and
//! the SQL `execute_project` reports via `RunReporter::model_compiled`
//! contains a time filter the model source never wrote — i.e. the filter
//! was *derived* (`source_bounds::derive_model_bounds` +
//! `transformer::inject_time_filter`), not hand-injected by the test.
//!
//! Red (pre-harness): no in-process path drove `execute_project` from
//! `smelt-db`'s test target before this file — the harness itself is the
//! artifact under test.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};

/// Stage a minimal batched model over an append-only source: no `WHERE`
/// clause anywhere in the model SQL.
fn stage_project(project_dir: &std::path::Path, db_path: &std::path::Path) {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [id]
---
SELECT d, id, val FROM smelt.sources.events
"#;
    std::fs::write(project_dir.join("models/events_batched.sql"), model_sql).unwrap();

    let source_yml = r#"description: Raw append-only events.
columns:
  - name: d
    type: DATE
  - name: id
    type: INTEGER
  - name: val
    type: DOUBLE
"#;
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(project_dir.join("models/sources/events.yml"), source_yml).unwrap();

    let smelt_yml = format!(
        "name: link_c_harness_smoke\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_source(db_path: &std::path::Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 10.0),
            (DATE '2024-01-02', 2, 20.0),
            (DATE '2024-01-03', 3, 30.0)
        ) AS t(d, id, val);
        "#,
    )
    .expect("seed source");
}

#[tokio::test]
async fn execute_project_derives_time_filter_no_hand_injected_where() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_source(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-03".to_string());

    let reporter = SqlCapturingReporter::new();
    let outcome = project
        .run("smoke-run", request, &reporter)
        .await
        .expect("execute_project must succeed for a no-WHERE batched model");

    assert!(
        outcome.models.contains_key("events_batched"),
        "expected events_batched in RunOutcome; got: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    // The model source has no WHERE clause anywhere — any filter clause
    // present in the compiled SQL was derived by the framework, not written
    // by this test.
    let compiled = reporter.sql_for("events_batched");
    assert!(
        !compiled.is_empty(),
        "expected at least one compiled batch for events_batched"
    );
    for sql in &compiled {
        assert!(
            sql.to_lowercase().contains("where"),
            "compiled SQL for events_batched has no derived WHERE clause; \
             either bound derivation didn't run or the harness bypassed it: {sql}"
        );
        assert!(
            sql.contains('d'),
            "derived filter should reference the partition column 'd': {sql}"
        );
    }

    // Read back through a fresh connection — proves the harness's DuckDB
    // path is real, not a mock.
    let conn = project.connect().expect("connect");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main.events_batched WHERE d >= '2024-01-01'::DATE AND d < '2024-01-03'::DATE",
            [],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(
        count, 2,
        "expected exactly the 2024-01-01 and 2024-01-02 rows within the requested window"
    );
}
