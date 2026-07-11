//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-02` (`docs/research/20260705-property-discovery-loop.md` §4;
//! `docs/plans/20260705-property-discovery-loop.md` phase F).
//!
//! Same additive `SUM` group-by / append-only shape as `G-01`
//! (`model_shapes::additive_agg_append_only`), but the adversarial dimension
//! is **re-delivery**: after a window is processed once, the exact same
//! `[start, end)` window is re-run — Link A's kind (2), "does re-running
//! double-count?" (design §2.1). The Link-0 table (§2.0) says `SUM`/`COUNT`
//! are non-idempotent monoids: a fold-delta technique that blindly *appends*
//! a delta on re-run would double the total. Whether smelt's actual emitted
//! batched maintenance is safe here is an execution-mechanism question, not
//! an algebra one — a prior research pass
//! (`crates/smelt-backend/src/lib.rs::resolve_strategy`,
//! `crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`)
//! confirmed batched refresh always emits `DELETE FROM table WHERE col IN
//! [start,end)` then `INSERT INTO table {sql}` inside one transaction — a
//! full partition *replace*, not an append and not a unique-key-scoped
//! upsert (`unique_key` is not consulted for strategy selection at all).
//! Re-running an identical window therefore has no ledger/dedup obligation to
//! violate: there is nothing to fold onto, only a fresh recompute. Predicted:
//! HOLDS — re-delivery is a no-op vs a single run, not a double-count.

use std::path::Path;

use proptest::prelude::*;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::model_shapes::{additive_agg_append_only, ModelShape};

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
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n{cols}"
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

fn create_empty_events(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.sources_events (d DATE, id INTEGER, val DOUBLE);
        "#,
    )
    .expect("create empty source table");
}

/// Independent full-refresh oracle: `SUM(val)` over the CURRENT full contents
/// of the source table for `date` — no smelt compilation, no derived filter.
fn full_refresh_total(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT SUM(val) FROM main.sources_events WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_total(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT total FROM main.events_daily_total_append_only WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

/// 1-3 rows for a single one-day window, whole-number values (same
/// `BigInt`-cast-truncation dodge as `G-01`'s `arb_disjoint_windows` — see its
/// doc comment; unrelated to this cell's re-delivery hypothesis).
fn arb_window_rows() -> impl Strategy<Value = Vec<f64>> {
    proptest::collection::vec((-50_i64..=50_i64).prop_map(|v| v as f64), 1..=3)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn redelivering_the_same_window_does_not_double_count_the_fold(
        vals in arb_window_rows(),
        redeliveries in 1_usize..=3
    ) {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let shape = additive_agg_append_only();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().to_path_buf();
            let db_path = project_dir.join("dev.duckdb");

            stage_project(&shape, &project_dir, &db_path);
            create_empty_events(&db_path);

            let project =
                LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

            let day = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
            let next_day = day + chrono::Duration::days(1);

            // Seed the window's rows once, before it is ever run.
            {
                let conn = project.connect().expect("connect for seed");
                for (i, val) in vals.iter().enumerate() {
                    conn.execute(
                        &format!(
                            "INSERT INTO main.sources_events VALUES (DATE '{}', {}, {:.6})",
                            day.format("%Y-%m-%d"),
                            i as i64 + 1,
                            val
                        ),
                        [],
                    )
                    .expect("seed window row");
                }
            }

            // Re-deliver the SAME [day, next_day) window `redeliveries` times
            // in a row — no new rows land between runs, no window advance.
            for i in 0..redeliveries {
                let mut request = base_request("dev");
                request.start = Some(day.format("%Y-%m-%d").to_string());
                request.end = Some(next_day.format("%Y-%m-%d").to_string());
                project
                    .run_quiet(&format!("run-{i}"), request)
                    .await
                    .expect("execute_project run must succeed");
            }

            let conn = project.connect().expect("connect after runs");
            let date_str = day.format("%Y-%m-%d").to_string();
            let maintained = maintained_total(&conn, &date_str);
            let full_refresh = full_refresh_total(&conn, &date_str);

            prop_assert_eq!(
                maintained,
                full_refresh,
                "additive SUM fold diverged from full-refresh after {} \
                 re-deliveries of the same window (day {}): vals={:?}",
                redeliveries,
                date_str,
                vals
            );

            Ok(())
        })?;
    }
}
