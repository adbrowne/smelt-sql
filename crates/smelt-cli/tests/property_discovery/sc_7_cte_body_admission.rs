//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-7` (`docs/research/20260707-property-partition-alignment.md` §6
//! gap 2 / §7; hypothesis B5 in `docs/research/property-discovery/catalog.jsonl`).
//!
//! Hypothesis: the batched HAVING/DISTINCT admission walks cover only the
//! outer UNION chain, and CTE bodies are deliberately exempt from the
//! subquery gate (the "CTE bypass") — so a **cross-partition `DISTINCT`
//! inside a CTE body** is judged by nobody and the model is ADMITTED even
//! though its CTE holds cross-partition state. A row appended into a later
//! partition then changes the CTE's dedup output for a key whose fact rows
//! live in an earlier, already-written partition; batched maintenance never
//! rewrites that earlier partition, so the maintained table diverges from a
//! full refresh. Expected RED = the admission hole (the rare fail-OPEN case);
//! GREEN = admission refuses the model (the per-scope walk judges the CTE
//! body's DISTINCT by the same rule as a top-level DISTINCT).

use std::path::Path;

use crate::link_c_harness::{base_request, LinkCProject};
use crate::model_shapes::{cte_cross_partition_distinct, ModelShape};

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
    let source_yml = format!(
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n{cols}timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n"
    );
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

fn seed_day_one(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 'basic')
        ) AS t(d, user_id, tier);
        "#,
    )
    .expect("seed sources");
}

/// Independent full-refresh oracle: the model's own CTE + join logic
/// re-expressed over the CURRENT full contents of the source table — the
/// number of output rows a full refresh would hold for `date`.
fn full_refresh_rows_for(conn: &duckdb::Connection, date: &str) -> i64 {
    conn.query_row(
        &format!(
            "WITH user_tiers AS (SELECT DISTINCT user_id, tier FROM main.sources_events)
             SELECT COUNT(*) FROM main.sources_events e
             JOIN user_tiers t ON e.user_id = t.user_id
             WHERE e.d = DATE '{date}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_rows_for(conn: &duckdb::Connection, date: &str) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM main.events_tiered WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

/// SC-7's owning test. GREEN = batched admission refuses the model (the
/// CTE-internal cross-partition DISTINCT is judged by the same per-scope
/// alignment rule as a top-level DISTINCT, fail-closed). RED = the model is
/// admitted; the run schedule then demonstrates the resulting divergence
/// from full refresh, and the test fails naming it.
#[tokio::test]
async fn cte_internal_cross_partition_distinct_is_refused() {
    let shape = cte_cross_partition_distinct();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_day_one(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02). enforce_safety mirrors the
    // CLI default — a refused model must surface as a hard error, never a
    // silent full-refresh downgrade.
    let mut request = base_request("dev");
    request.enforce_safety = true;
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());

    let run1 = project.run_quiet("run-1", request.clone()).await;

    let Err(err) = run1 else {
        // Fail-open admission hole still present: demonstrate the divergence
        // the admitted model produces, then fail naming it.
        {
            let conn = project.connect().expect("connect for late append");
            conn.execute(
                "INSERT INTO main.sources_events VALUES (DATE '2024-01-02', 1, 'premium')",
                [],
            )
            .expect("append late row into the next partition");
        }

        let mut request2 = base_request("dev");
        request2.enforce_safety = true;
        request2.start = Some("2024-01-02".to_string());
        request2.end = Some("2024-01-03".to_string());
        project
            .run_quiet("run-2", request2)
            .await
            .expect("run 2 must succeed while the model is admitted");

        let conn = project.connect().expect("connect after run 2");
        let maintained_day1 = maintained_rows_for(&conn, "2024-01-01");
        let full_refresh_day1 = full_refresh_rows_for(&conn, "2024-01-01");

        panic!(
            "LATENT BUG (SC-7 confirmed): a batched model with a cross-partition DISTINCT \
             inside a CTE body was ADMITTED (run 1 succeeded); after a late row appended into \
             the NEXT partition, the already-written 2024-01-01 partition holds \
             {maintained_day1} row(s) where the full-refresh oracle holds {full_refresh_day1} \
             — batched maintenance never rewrites the stale partition, so the divergence is \
             permanent. Admission must refuse this model."
        );
    };

    let msg = format!("{err:#}");
    assert!(
        msg.contains("DISTINCT"),
        "refusal must name the DISTINCT admission gate; got: {msg}"
    );
    assert!(
        msg.contains("Incremental safety check refused"),
        "refusal must surface through the planner safety gate; got: {msg}"
    );
}
