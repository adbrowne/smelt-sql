//! Cell `G-10` (`docs/research/property-discovery/catalog.md`;
//! `docs/plans/20260707-maintenance-plan-impl.md` Phase MP10) — permanent
//! regression fixture, graduated out of the disposable-probe lane.
//!
//! `join_shape::JoinContext`/`fan_out` (`crates/smelt-logical/src/analysis/join_shape.rs`)
//! now expresses a genuine COMPOSITE unique key (jointly, but not
//! individually, unique columns) — see `crates/smelt-logical/tests/join_shape_composite.rs`
//! for the unit-level proof. This test is the end-to-end companion: a real
//! fact/dim project, staged and executed through `execute_project` via
//! `LinkCProject`, where `dims` is unique only on the PAIR `(user_id, dt)`
//! (neither column alone is unique — every `user_id` repeats across several
//! `dt`s and vice versa). It asserts smelt's maintained batched output for
//! the composite-key-joined enrichment matches a hand-written full-refresh
//! oracle doing the same composite join over the current source contents.
//!
//! Not tagged `EXPERIMENTAL(property-discovery): disposable` — this is
//! standing testkit coverage, not a throwaway probe. The original disposable
//! ground-truth proptest,
//! `crates/smelt-db/tests/proptests/maintenance_link_b_composite_key_fan_out.rs`,
//! remains unchanged: its own assertion uses an UNDECLARED key
//! (`JoinContext::new()`), so it is unaffected by the composite-key API
//! existing now.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use smelt_maintenance_testkit::model_shapes::{join_composite_key_fan_out, MultiSourceModelShape};
use smelt_maintenance_testkit::oracle::multiset_equal;

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

/// `dims` rows: `(user_id, dt, payload)` pairs are unique as a PAIR, but
/// neither `user_id` nor `dt` alone is unique — `user_id=1` appears under
/// `dt=100` and `dt=200`; `dt=100` appears under `user_id=1` and
/// `user_id=2`. Only the composite `(user_id, dt)` equality is one-to-one.
fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_facts AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 100, 10.0),
            (DATE '2024-01-01', 2, 200, 20.0)
        ) AS t(d, user_id, dt, val);
        CREATE OR REPLACE TABLE main.sources_dims AS
        SELECT * FROM (VALUES
            (1, 100, 111),
            (1, 200, 222),
            (2, 100, 333),
            (2, 200, 444)
        ) AS t(user_id, dt, payload);
        "#,
    )
    .expect("seed sources");
}

/// Full-refresh oracle: the model's own composite join, re-expressed over
/// the CURRENT full contents of both staged source tables.
fn full_refresh_sql() -> &'static str {
    "SELECT f.d, f.user_id, f.dt, f.val, dm.payload
     FROM main.sources_facts f
     JOIN main.sources_dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt
     WHERE f.d = DATE '2024-01-01'"
}

fn maintained_sql() -> &'static str {
    "SELECT d, user_id, dt, val, payload FROM main.facts_enriched WHERE d = DATE '2024-01-01'"
}

#[tokio::test]
async fn composite_key_join_enrichment_matches_full_refresh_oracle() {
    let shape = join_composite_key_fan_out();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    let reporter = SqlCapturingReporter::new();
    project
        .run("run-1", request, &reporter)
        .await
        .expect("run must succeed");

    let conn = project.connect().expect("connect after run");
    let compiled_sql = reporter.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert!(
        multiset_equal(&conn, maintained_sql(), full_refresh_sql()),
        "smelt's maintained enrichment for the composite-key (user_id, dt) join diverges from \
         the full-refresh oracle. Compiled SQL: {compiled_sql:?}"
    );

    // Sanity: confirm the fixture is a genuine composite-key case — neither
    // user_id nor dt is unique alone in dims, only the pair is.
    let max_per_user_id: i64 = conn
        .query_row(
            "SELECT MAX(cnt) FROM (SELECT user_id, COUNT(*) AS cnt FROM main.sources_dims GROUP BY user_id) t",
            [],
            |row| row.get(0),
        )
        .expect("dims non-empty");
    assert!(
        max_per_user_id > 1,
        "fixture sanity: user_id alone must NOT be unique in dims (got max count {max_per_user_id})"
    );
    let max_per_dt: i64 = conn
        .query_row(
            "SELECT MAX(cnt) FROM (SELECT dt, COUNT(*) AS cnt FROM main.sources_dims GROUP BY dt) t",
            [],
            |row| row.get(0),
        )
        .expect("dims non-empty");
    assert!(
        max_per_dt > 1,
        "fixture sanity: dt alone must NOT be unique in dims (got max count {max_per_dt})"
    );
}
