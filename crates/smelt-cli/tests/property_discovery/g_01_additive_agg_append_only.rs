//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-01` (`docs/research/20260705-property-discovery-loop.md` §4;
//! `docs/plans/20260705-property-discovery-loop.md` phase F).
//!
//! Control cell establishing the fold-delta happy path: an additive `SUM`
//! group-by, batched per partition (`unique_key: [d]`), over a source
//! declared `mutation_profile: append_only`. Each partition's rows are fully
//! present *before* its window is ever requested, and no already-processed
//! partition is ever mutated or re-delivered afterwards — the disjoint-delta
//! case the paper's admission-matrix theorem predicts HOLDS unconditionally
//! for an additive monoid. Predicted: no divergence, over any number of
//! disjoint append-only windows with any row values.

use std::path::Path;

use proptest::prelude::*;

use crate::link_c_harness::{base_request, LinkCProject};
use crate::model_shapes::{additive_agg_append_only, ModelShape};

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
    // Declares `mutation_profile: append_only` — the world-fact this cell's
    // control hypothesis is about (contrast `SC-2`'s `mutable` declaration
    // over the same SQL shape).
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

/// A schedule for this cell: 2-4 disjoint one-day windows, each with 1-3 rows
/// whose values are proptest-generated. Every window's rows are inserted
/// BEFORE that window is ever requested (append-only, no lateness, no
/// re-delivery) — the disjoint-delta control shape.
///
/// Values include genuine fractional `f64`s. The whole-number workaround
/// that previously lived here (single-segment scan-root sources were
/// silently dropped from the `TypeContext`, so `SUM(val)` fell back to a
/// truncating `CAST(… AS BIGINT)` — ledger `G-01` note) is gone: design
/// fork F4 fixed `add_source_info_to_type_context` to register
/// single-segment sources under the schema-free simple key, pinned by
/// `crates/smelt-db/tests/single_segment_source_types.rs`. Fractional
/// values here are the end-to-end assertion that the fix holds through the
/// real path — a regression re-truncates the sums and fails the oracle.
/// Halves (`v as f64 / 2.0`) keep the arithmetic float-exact so the oracle
/// stays an equality check, while guaranteeing non-integer sums occur.
fn arb_disjoint_windows() -> impl Strategy<Value = Vec<Vec<f64>>> {
    proptest::collection::vec(
        proptest::collection::vec((-101_i64..=101_i64).prop_map(|v| v as f64 / 2.0), 1..=3),
        2..=4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn additive_sum_fold_over_disjoint_append_only_windows_matches_full_refresh(
        windows in arb_disjoint_windows()
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

            let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
            let mut next_id = 1_i64;

            for (i, vals) in windows.iter().enumerate() {
                let day = base + chrono::Duration::days(i as i64);
                let next_day = day + chrono::Duration::days(1);

                // Append this window's rows BEFORE the window is run — no
                // lateness, no revisit of an already-processed partition.
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

                let mut request = base_request("dev");
                request.start = Some(day.format("%Y-%m-%d").to_string());
                request.end = Some(next_day.format("%Y-%m-%d").to_string());
                project
                    .run_quiet(&format!("run-{i}"), request)
                    .await
                    .expect("execute_project run must succeed");

                if vals.is_empty() {
                    continue;
                }

                let conn = project.connect().expect("connect after run");
                let date_str = day.format("%Y-%m-%d").to_string();
                let maintained = maintained_total(&conn, &date_str);
                let full_refresh = full_refresh_total(&conn, &date_str);

                prop_assert_eq!(
                    maintained,
                    full_refresh,
                    "additive SUM fold diverged from full-refresh for a disjoint \
                     append-only partition (day {}): windows={:?}",
                    date_str,
                    windows
                );
            }

            Ok(())
        })?;
    }
}
