//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-07` (`docs/research/20260705-property-discovery-loop.md` §4;
//! `docs/plans/20260705-property-discovery-loop.md` phase F).
//!
//! Same source shape as `G-01`/`G-03` (`events(d, id, val)`, `mutation_profile:
//! append_only`), but the combiners are `MEDIAN(val)` and exact
//! `COUNT(DISTINCT id)` — **holistic / non-monoid** (Link 0 table §2.0: no
//! bounded combiner state; `combiner_discriminants` classifies both into its
//! fail-closed `holistic_or_unknown()` bucket,
//! `crates/smelt-logical/src/analysis/discriminants.rs`). The catalog's
//! hypothesis predicted CONDITIONAL/REFUTED on the theory that smelt might
//! attempt a bounded "fold-delta" state for a holistic combiner and get it
//! wrong (no bounded state exists to fold into).
//!
//! Research established that hypothesis's premise is false for `refresh:
//! batched`: `combiner_discriminants`/`Discriminants` is consumed only by the
//! cumulative/running-total feature (`rules/cumulative.rs`) and `join_shape`'s
//! fan-out analysis — `rules/incremental.rs`, which governs batched GROUP BY
//! models, never imports or consults it. The actual batched technique is
//! **recompute-region**, not fold-delta: every batch is `DELETE [start,end)` +
//! `INSERT` a **freshly recomputed** `SELECT ... GROUP BY d` for that window
//! (`crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`),
//! re-scanning ALL current source rows in the partition on every run — there
//! is no partial aggregate state to fold, so it is combiner-identity-agnostic
//! by construction (same finding `G-03`'s doc comment already made for `MAX`,
//! now extended to genuinely holistic combiners). Predicted (revised): HOLDS
//! unconditionally, including under re-delivery (re-delivery is a full
//! recompute of the same window, not a fold onto remembered state, so even a
//! non-idempotent-under-folding combiner like exact `COUNT(DISTINCT ...)`
//! cannot double-count).
//!
//! Duplicate `id` values are seeded WITHIN a window (not globally unique like
//! `G-01`/`G-03`'s `next_id`) so `COUNT(DISTINCT id)` genuinely differs from
//! `COUNT(id)` — otherwise this cell would not actually exercise the holistic
//! combiner's defining property (multiset de-duplication) at all.

use std::path::Path;

use proptest::prelude::*;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::model_shapes::{holistic_agg_append_only, ModelShape};

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

/// Independent full-refresh oracle: `MEDIAN(val)`/`COUNT(DISTINCT id)` over the
/// CURRENT full contents of the source table for `date` — no smelt
/// compilation, no derived filter.
fn full_refresh_holistic(conn: &duckdb::Connection, date: &str) -> (f64, i64) {
    conn.query_row(
        &format!(
            "SELECT MEDIAN(val), COUNT(DISTINCT id) FROM main.sources_events WHERE d = DATE '{date}'"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("full-refresh oracle query")
}

fn maintained_holistic(conn: &duckdb::Connection, date: &str) -> (f64, i64) {
    conn.query_row(
        &format!(
            "SELECT med_val, distinct_ids FROM main.events_daily_holistic_append_only WHERE d = DATE '{date}'"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("maintained-table read")
}

/// 2-4 disjoint one-day windows, each with 2-5 `(id, val)` rows and a 0-2
/// re-delivery count for that window once processed. `id` is drawn from a
/// SMALL range (1..=3) so duplicate ids within a window are common — the
/// point of this cell is exercising `COUNT(DISTINCT id)`'s de-duplication,
/// which a global-unique-id scheme (`G-01`/`G-03`'s `next_id`) would never
/// touch. `val` uses the same whole-number-cast-to-f64 dodge as `G-01`/`G-03`
/// (see their doc comments) — unrelated to this cell's hypothesis, sidesteps
/// an orthogonal already-discovered `BigInt`-truncation bug.
fn arb_disjoint_windows_with_redelivery() -> impl Strategy<Value = Vec<(Vec<(i64, f64)>, usize)>> {
    proptest::collection::vec(
        (
            proptest::collection::vec(
                (1_i64..=3, (-50_i64..=50_i64).prop_map(|v| v as f64)),
                2..=5,
            ),
            0_usize..=2,
        ),
        2..=4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn holistic_median_and_count_distinct_fold_over_disjoint_append_only_windows_with_redelivery_matches_full_refresh(
        windows in arb_disjoint_windows_with_redelivery()
    ) {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let shape = holistic_agg_append_only();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().to_path_buf();
            let db_path = project_dir.join("dev.duckdb");

            stage_project(&shape, &project_dir, &db_path);
            create_empty_events(&db_path);

            let project =
                LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

            let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

            for (i, (rows, redeliveries)) in windows.iter().enumerate() {
                let day = base + chrono::Duration::days(i as i64);
                let next_day = day + chrono::Duration::days(1);

                // Append this window's rows BEFORE the window is ever run — no
                // lateness, no revisit of an already-processed partition with
                // NEW rows (only re-delivery of the same request, below).
                {
                    let conn = project.connect().expect("connect for seed");
                    for (id, val) in rows {
                        conn.execute(
                            &format!(
                                "INSERT INTO main.sources_events VALUES (DATE '{}', {}, {:.6})",
                                day.format("%Y-%m-%d"),
                                id,
                                val
                            ),
                            [],
                        )
                        .expect("seed window row");
                    }
                }

                if rows.is_empty() {
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
                let maintained = maintained_holistic(&conn, &date_str);
                let full_refresh = full_refresh_holistic(&conn, &date_str);

                prop_assert_eq!(
                    maintained,
                    full_refresh,
                    "holistic MEDIAN/COUNT(DISTINCT) fold diverged from full-refresh for a \
                     disjoint append-only partition after {} re-deliveries (day {}): windows={:?}",
                    redeliveries,
                    date_str,
                    windows
                );
            }

            Ok(())
        })?;
    }
}
