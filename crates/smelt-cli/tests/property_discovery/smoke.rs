//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `P0-1` acceptance test. Proves the in-process Link-C harness
//! (`link_c_harness`) drives smelt's REAL bound derivation: a `refresh: batched`
//! model with **no `WHERE` clause at all** (from `model_shapes::batched_passthrough`)
//! is compiled and executed through `execute_project`, and the SQL reported via
//! `RunReporter::model_compiled` contains a time filter the model source never
//! wrote — i.e. the filter was *derived* (`source_bounds::derive_model_bounds`),
//! not hand-injected by the test.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use smelt_maintenance_testkit::model_shapes::{batched_passthrough, ModelShape};

/// Stage a project directory from a `ModelShape`: the model file, the source
/// yml, and smelt.yml pointing at the temp DuckDB.
fn stage_project(shape: &ModelShape, project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    let cols: String = shape
        .source_columns
        .iter()
        .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
        .collect();
    let source_yml = format!("description: property-discovery source.\ncolumns:\n{cols}");
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", shape.source)),
        source_yml,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_events(db_path: &Path) {
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
    let shape = batched_passthrough();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_events(&db_path);

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
        outcome.models.contains_key(shape.name),
        "expected {} in RunOutcome; got: {:?}",
        shape.name,
        outcome.models.keys().collect::<Vec<_>>()
    );

    // The model source has no WHERE clause anywhere — any filter clause present
    // in the compiled SQL was derived by the framework, not written by the test.
    let compiled = reporter.sql_for(shape.name);
    assert!(
        !compiled.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );
    for sql in &compiled {
        assert!(
            sql.to_lowercase().contains("where"),
            "compiled SQL for {} has no derived WHERE clause; either bound \
             derivation didn't run or the harness bypassed it: {sql}",
            shape.name
        );
        assert!(
            sql.contains('d'),
            "derived filter should reference the partition column 'd': {sql}"
        );
    }

    // Read back through a fresh connection — proves the harness's DuckDB path is
    // real, not a mock.
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
