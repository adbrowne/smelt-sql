//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-1` (`docs/research/20260705-property-discovery-loop.md` §4, §2.3;
//! `docs/plans/20260705-property-discovery-loop.md` phase E).
//!
//! Hypothesis: `source_bounds`'s "no temporal dependency" fallback yields an
//! optimistic zero-margin bound for the correlated `EXISTS` construct
//! (`model_shapes::correlated_exists_attribution`), so smelt's derived read
//! filter on `conversions` clamps away a late conversion appended *between*
//! two runs, within its declared 7-day attribution window — the maintained
//! `converted` column would stay `false` where full-refresh (over the source
//! state at the final step) says `true`.
//!
//! Reachability (design §2.1 coverage caveat, N4): the late conversion is
//! appended directly to `main.sources_conversions` between two
//! `execute_project` runs, never pre-populated — a pre-populated conversion
//! would be visible to both the incremental and full-refresh paths and mask
//! the bug this cell hunts.

use std::path::Path;

use crate::shapes::{correlated_exists_attribution, MultiSourceModelShape};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};

fn stage_project(shape: &MultiSourceModelShape, project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    for src in shape.sources {
        let cols: String = src
            .columns
            .iter()
            .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
            .collect();
        let ts_block = match src.timeseries {
            Some((event_time_col, partition_col)) => format!(
                "timeseries:\n  event_time_column: {event_time_col}\n  partition_column: {partition_col}\n  granularity: day\n"
            ),
            None => String::new(),
        };
        let source_yml =
            format!("description: property-discovery source.\ncolumns:\n{cols}{ts_block}");
        std::fs::write(
            project_dir.join(format!("models/sources/{}.yml", src.name)),
            source_yml,
        )
        .unwrap();
    }

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1)
        ) AS t(event_date, user_id);
        CREATE OR REPLACE TABLE main.sources_conversions AS
        SELECT * FROM (VALUES (CAST(NULL AS BIGINT), CAST(NULL AS DATE))) AS t(user_id, conversion_date)
        WHERE FALSE;
        "#,
    )
    .expect("seed sources");
}

/// The independent full-refresh oracle: the model's own correlated-`EXISTS`
/// logic, re-expressed as a plain query over the CURRENT full contents of
/// both staged source tables — no smelt compilation, no derived filter. This
/// is "full-refresh over the source state at step k" (design N3) for this
/// specific model shape.
fn full_refresh_converted(conn: &duckdb::Connection, user_id: i64, event_date: &str) -> bool {
    conn.query_row(
        &format!(
            "SELECT EXISTS(
                SELECT 1 FROM main.sources_conversions c
                WHERE c.user_id = e.user_id
                  AND c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL 7 DAY
            ) FROM main.sources_events e
            WHERE e.user_id = {user_id} AND e.event_date = DATE '{event_date}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_converted(conn: &duckdb::Connection, user_id: i64, event_date: &str) -> bool {
    conn.query_row(
        &format!(
            "SELECT converted FROM main.event_conversions
             WHERE user_id = {user_id} AND event_date = DATE '{event_date}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn late_conversion_appended_between_runs_within_7_day_window() {
    let shape = correlated_exists_attribution();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) with no matching conversion yet.
    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    let reporter = SqlCapturingReporter::new();
    project
        .run("run-1", request.clone(), &reporter)
        .await
        .expect("run 1 must succeed");

    {
        let conn = project.connect().expect("connect after run 1");
        assert!(
            !maintained_converted(&conn, 1, "2024-01-01"),
            "no conversion exists yet at run 1 — converted must be false"
        );
    }

    // Between runs: append a late conversion directly to the source table —
    // 2 days after the event, well inside the declared 7-day window. This
    // row was ABSENT at run 1 and arrives only now (design §2.1 reachability
    // rule): a pre-populated conversion would be visible to run 1 too and
    // mask the bug this cell hunts.
    {
        let conn = project.connect().expect("connect for late append");
        conn.execute(
            "INSERT INTO main.sources_conversions VALUES (1, DATE '2024-01-03')",
            [],
        )
        .expect("append late conversion");
    }

    // Run 2: re-run (backfill) the SAME window. A batched `unique_key` upsert
    // re-materializes the same partition — this isolates the source_bounds
    // read-filter question (does the re-derived `conversions` read see the
    // late row?) from the separate, expected limitation that a plain
    // forward-only advance would never revisit an already-closed window.
    let reporter2 = SqlCapturingReporter::new();
    project
        .run("run-2", request, &reporter2)
        .await
        .expect("run 2 must succeed");

    let conn = project.connect().expect("connect after run 2");
    let maintained = maintained_converted(&conn, 1, "2024-01-01");
    let full_refresh = full_refresh_converted(&conn, 1, "2024-01-01");

    let conversions_sql = reporter2.sql_for(shape.name);
    assert!(
        !conversions_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert_eq!(
        maintained, full_refresh,
        "LATENT BUG (SC-1 confirmed): smelt's maintained `converted` ({maintained}) \
         diverges from the full-refresh oracle ({full_refresh}) after a late \
         conversion landed within the declared 7-day window between runs. \
         Compiled SQL for run 2: {conversions_sql:?}"
    );
    assert!(
        full_refresh,
        "oracle sanity: the late conversion IS within the 7-day window, so the \
         full-refresh oracle itself must say true (test setup bug otherwise)"
    );
}
