//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-1b` (`docs/research/property-discovery/catalog.jsonl`; appended
//! from `SC-1`). `FIX-1` scoped `extract_form_b_bounds`'s Form-B pattern
//! matching to the LHS column *name* (`lhs_column_is_partition_col`), closing
//! the cross-source misattribution that `SC-1` found (a pattern legitimately
//! constraining `conversions` was also attributed to the unrelated `events`).
//! But `derive_bound_for_source` (`source_bounds.rs`) is still invoked once
//! per source with only that source's own partition-column *name* — it has no
//! notion of which FROM/JOIN alias belongs to which source. If two sources
//! declare a partition column of the SAME name, a Form-B pattern scoped by
//! alias to one source (`r.d BETWEEN ...`) still satisfies the name-only LHS
//! check for the OTHER source too.
//!
//! `model_shapes::column_name_collision_across_sources`: `logins` (partition
//! col `d`) has no temporal-lookback pattern of its own; `resets` (partition
//! col also `d`) is the only source the correlated `EXISTS` predicate
//! constrains (`r.d BETWEEN l.d AND l.d + INTERVAL '3 days'`). This cell asks
//! whether the spurious cross-source match ever *narrows* `logins`'s derived
//! bound (unsound — would clamp away rows) or only ever *widens* it
//! (over-conservative, safe) via `BoundResult::merge`'s max-merge
//! (`source_bounds.rs::BoundResult::merge` takes `before.max`/`after.max` —
//! merging in a spurious match can only add margin, never remove it).

use std::path::Path;

use crate::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use crate::model_shapes::{column_name_collision_across_sources, MultiSourceModelShape};

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
        CREATE OR REPLACE TABLE main.sources_logins AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1)
        ) AS t(d, user_id);
        CREATE OR REPLACE TABLE main.sources_resets AS
        SELECT * FROM (VALUES (CAST(NULL AS BIGINT), CAST(NULL AS DATE))) AS t(user_id, d)
        WHERE FALSE;
        "#,
    )
    .expect("seed sources");
}

/// Full-refresh oracle over the CURRENT contents of both staged source
/// tables — no smelt compilation, no derived filter (design N3).
fn full_refresh_reset_flag(conn: &duckdb::Connection, user_id: i64, d: &str) -> bool {
    conn.query_row(
        &format!(
            "SELECT EXISTS(
                SELECT 1 FROM main.sources_resets r
                WHERE r.user_id = l.user_id
                  AND r.d BETWEEN l.d AND l.d + INTERVAL 3 DAY
            ) FROM main.sources_logins l
            WHERE l.user_id = {user_id} AND l.d = DATE '{d}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_reset_flag(conn: &duckdb::Connection, user_id: i64, d: &str) -> bool {
    conn.query_row(
        &format!(
            "SELECT reset_flag FROM main.logins_with_reset_flag
             WHERE user_id = {user_id} AND d = DATE '{d}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn same_named_partition_column_collision_only_widens_never_narrows() {
    let shape = column_name_collision_across_sources();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) with no reset row yet.
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
            !maintained_reset_flag(&conn, 1, "2024-01-01"),
            "no reset exists yet at run 1 — reset_flag must be false"
        );
    }

    // Between runs: append a late reset directly to the source table, 2 days
    // after the login, inside the declared 3-day window. Absent at run 1
    // (design §2.1 reachability rule) — a pre-populated reset would be seen
    // by both paths and mask the question this cell asks.
    {
        let conn = project.connect().expect("connect for late append");
        conn.execute(
            "INSERT INTO main.sources_resets VALUES (1, DATE '2024-01-03')",
            [],
        )
        .expect("append late reset");
    }

    // Run 2: re-run (backfill) the SAME window.
    let reporter2 = SqlCapturingReporter::new();
    project
        .run("run-2", request, &reporter2)
        .await
        .expect("run 2 must succeed");

    let conn = project.connect().expect("connect after run 2");
    let maintained = maintained_reset_flag(&conn, 1, "2024-01-01");
    let full_refresh = full_refresh_reset_flag(&conn, 1, "2024-01-01");

    assert!(
        full_refresh,
        "oracle sanity: the late reset IS within the 3-day window, so the \
         full-refresh oracle itself must say true (test setup bug otherwise)"
    );
    assert_eq!(
        maintained, full_refresh,
        "hypothesis REFUTED-as-unsound would require this to diverge: \
         maintained reset_flag ({maintained}) vs full-refresh ({full_refresh}). \
         A divergence here would mean the column-name collision NARROWED \
         logins's derived bound — an actual unsound acceptance, not merely a \
         spurious over-read."
    );

    // Link-B evidence: the model's own compiled SQL (which inlines the
    // `logins` source read) should show the spurious 3-day widen on `d` even
    // though `logins` has no Form-B pattern of its own — the over-conservative
    // (safe) finding, distinct from the unsound-narrowing question above.
    let compiled_sql = reporter2.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );
    let upper = compiled_sql.join("\n").to_uppercase();
    assert!(
        upper.contains("2024-01-05"),
        "Link-B finding expected: `logins`'s read should be spuriously widened \
         by 3 days (to 2024-01-05, the run-2 end '2024-01-02' + 3d) due to the \
         same-named-column collision with `resets`'s own pattern, even though \
         `logins` has no Form-B pattern of its own. Compiled SQL: {compiled_sql:?}"
    );
}
