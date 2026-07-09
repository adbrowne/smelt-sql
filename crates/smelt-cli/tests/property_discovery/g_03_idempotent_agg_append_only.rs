//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-03` (`docs/research/20260705-property-discovery-loop.md` §4;
//! `docs/plans/20260705-property-discovery-loop.md` phase F).
//!
//! Same source shape as `G-01`/`G-02` (`events(d, id, val)`, `mutation_profile:
//! append_only`), but the combiner is `MAX` — an idempotent monoid (Link 0
//! table §2.0), unlike `SUM`'s additive one. The hypothesis (design §4 `G-03`)
//! is that an idempotent-monoid fold over append-only deltas equals
//! full-refresh under ALL schedules, INCLUDING re-delivery: `G-02` already
//! established that smelt's batched refresh never folds onto remembered
//! state (`DELETE [start,end)` + `INSERT` full partition replace,
//! `crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`),
//! so re-delivery is a no-op recompute regardless of combiner. For an
//! idempotent combiner specifically, even a hypothetical fold-onto-remembered-
//! state technique would still be safe under re-delivery (idempotent: folding
//! the same delta twice is the identity), so this cell doubles as a check that
//! smelt does not do something needlessly UNSOUND for the easier case. This
//! test therefore exercises BOTH adversarial dimensions in one schedule:
//! disjoint append-only windows (`G-01`'s shape) each followed by 0-2
//! re-deliveries (`G-02`'s shape). Predicted: HOLDS unconditionally.

use std::path::Path;

use proptest::prelude::*;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::model_shapes::{idempotent_agg_append_only, ModelShape};

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

/// Independent full-refresh oracle: `MAX(val)` over the CURRENT full contents
/// of the source table for `date` — no smelt compilation, no derived filter.
fn full_refresh_max(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT MAX(val) FROM main.sources_events WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_max(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT max_val FROM main.events_daily_max_append_only WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

/// 2-4 disjoint one-day windows, each with 1-3 whole-number row values (same
/// `BigInt`-cast-truncation dodge as `G-01`'s `arb_disjoint_windows` — see its
/// doc comment; unrelated to this cell's hypothesis) and a 0-2 re-delivery
/// count for that window once processed.
fn arb_disjoint_windows_with_redelivery() -> impl Strategy<Value = Vec<(Vec<f64>, usize)>> {
    proptest::collection::vec(
        (
            proptest::collection::vec((-50_i64..=50_i64).prop_map(|v| v as f64), 1..=3),
            0_usize..=2,
        ),
        2..=4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn idempotent_max_fold_over_disjoint_append_only_windows_with_redelivery_matches_full_refresh(
        windows in arb_disjoint_windows_with_redelivery()
    ) {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let shape = idempotent_agg_append_only();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().to_path_buf();
            let db_path = project_dir.join("dev.duckdb");

            stage_project(&shape, &project_dir, &db_path);
            create_empty_events(&db_path);

            let project =
                LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

            let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
            let mut next_id = 1_i64;

            for (i, (vals, redeliveries)) in windows.iter().enumerate() {
                let day = base + chrono::Duration::days(i as i64);
                let next_day = day + chrono::Duration::days(1);

                // Append this window's rows BEFORE the window is ever run — no
                // lateness, no revisit of an already-processed partition with
                // NEW rows (only re-delivery of the same request, below).
                {
                    let conn = project.connect().expect("connect for seed");
                    for val in vals {
                        conn.execute(
                            &format!(
                                "INSERT INTO main.sources_events VALUES (DATE '{}', {}, {:.6})",
                                day.format("%Y-%m-%d"),
                                next_id,
                                val
                            ),
                            [],
                        )
                        .expect("seed window row");
                        next_id += 1;
                    }
                }

                if vals.is_empty() {
                    continue;
                }

                // Run once, then re-deliver the identical [day, next_day)
                // window `redeliveries` more times with no new rows landing
                // between runs.
                for run_i in 0..=*redeliveries {
                    let mut request = base_request("dev");
                    request.start = Some(day.format("%Y-%m-%d").to_string());
                    request.end = Some(next_day.format("%Y-%m-%d").to_string());
                    project
                        .run_quiet(&format!("run-{i}-{run_i}"), request)
                        .await
                        .expect("execute_project run must succeed");
                }

                let conn = project.connect().expect("connect after runs");
                let date_str = day.format("%Y-%m-%d").to_string();
                let maintained = maintained_max(&conn, &date_str);
                let full_refresh = full_refresh_max(&conn, &date_str);

                prop_assert_eq!(
                    maintained,
                    full_refresh,
                    "idempotent MAX fold diverged from full-refresh for a disjoint \
                     append-only partition after {} re-deliveries (day {}): windows={:?}",
                    redeliveries,
                    date_str,
                    windows
                );
            }

            Ok(())
        })?;
    }
}
