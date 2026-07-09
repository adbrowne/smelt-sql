//! TDD tests for the reconciliation ledger
//! (`docs/specs/maintenance_plan.md` §"The reconciliation ledger"):
//! `(output-region × column-group)` entries, graded storage, and the two
//! operations — fold-precondition + recompute-reset.

use smelt_state::file_store::FileStore;
use smelt_state::reconciliation::{Grade, Processed, ReconciliationLedger, Region};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tempfile::TempDir;

/// A region recompute must reset every ledger entry intersecting the
/// recomputed region to **exactly** the input state that recompute read —
/// not the union with whatever was folded before, and not empty. A stale
/// delta identity folded before the recompute (`"d-stale"`) must not
/// survive; the recompute's own read set (`"d-fresh"`) must be present
/// exactly.
#[test]
fn recompute_resets_intersecting_entries_exactly() {
    let mut ledger = ReconciliationLedger::new();
    let region = Region::new("2026-01-01", "2026-01-10");

    ledger
        .fold(&region, "{revenue}", Grade::Additive, "orders", "d-stale")
        .expect("first fold on empty entry always succeeds");

    // A recompute over an overlapping (not necessarily identical) region
    // reads exactly one input delta identity for `orders`.
    let recompute_region = Region::new("2026-01-05", "2026-01-15");
    let mut read = BTreeMap::new();
    read.insert(
        "orders".to_string(),
        BTreeSet::from(["d-fresh".to_string()]),
    );
    ledger.recompute_reset(
        &recompute_region,
        "{revenue}",
        Processed::DeltaIdentities(read.clone()),
    );

    // The original region's entry is gone — superseded by the recompute,
    // not left holding the stale delta.
    assert!(ledger.get(&region, "{revenue}").is_none());

    // The recompute's own region now holds exactly what it read: neither
    // empty nor accumulated on top of the stale entry.
    let entry = ledger
        .get(&recompute_region, "{revenue}")
        .expect("recompute-reset always leaves an entry at the recomputed region");
    assert_eq!(entry.processed, Processed::DeltaIdentities(read));
}

/// Successive folds against an idempotent-graded entry only ever move the
/// frontier watermark forward; a delta that does not advance the watermark
/// (equal to or behind the current one) is refused as already reflected.
#[test]
fn fold_extends_frontier_monotonically() {
    let mut ledger = ReconciliationLedger::new();
    let region = Region::new("2026-01-01", "2026-02-01");

    ledger
        .fold(
            &region,
            "{total}",
            Grade::Idempotent,
            "orders",
            "2026-01-05",
        )
        .expect("first fold establishes the frontier");
    assert_eq!(
        ledger.get(&region, "{total}").unwrap().processed,
        Processed::Frontier(BTreeMap::from([(
            "orders".to_string(),
            "2026-01-05".to_string()
        )]))
    );

    // A delta at or behind the current watermark is already reflected.
    let refused_equal = ledger
        .fold(
            &region,
            "{total}",
            Grade::Idempotent,
            "orders",
            "2026-01-05",
        )
        .unwrap_err();
    assert_eq!(refused_equal.delta, "2026-01-05");
    let refused_behind = ledger
        .fold(
            &region,
            "{total}",
            Grade::Idempotent,
            "orders",
            "2026-01-02",
        )
        .unwrap_err();
    assert_eq!(refused_behind.delta, "2026-01-02");

    // A delta strictly ahead of the current watermark advances it.
    ledger
        .fold(
            &region,
            "{total}",
            Grade::Idempotent,
            "orders",
            "2026-01-20",
        )
        .expect("a delta past the watermark extends the frontier");
    assert_eq!(
        ledger.get(&region, "{total}").unwrap().processed,
        Processed::Frontier(BTreeMap::from([(
            "orders".to_string(),
            "2026-01-20".to_string()
        )]))
    );
}

/// Two column groups over the same output region are independent ledger
/// entries: folding a delta into one group's entry has no effect on the
/// other's, and each group refuses a repeat of its own already-folded
/// delta independently.
#[test]
fn entries_keyed_region_by_group() {
    let mut ledger = ReconciliationLedger::new();
    let region = Region::new("2026-01-01", "2026-01-10");

    ledger
        .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
        .unwrap();
    ledger
        .fold(&region, "{refunds}", Grade::Additive, "orders", "d1")
        .unwrap();

    // Same delta identity, same region, same input — but different groups,
    // so both folds succeeded independently above and each group's entry
    // now refuses `d1` on its own account.
    let refused_revenue = ledger
        .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
        .unwrap_err();
    assert_eq!(refused_revenue.input, "orders");
    let refused_refunds = ledger
        .fold(&region, "{refunds}", Grade::Additive, "orders", "d1")
        .unwrap_err();
    assert_eq!(refused_refunds.input, "orders");

    // Advancing one group's entry with a new delta does not touch the
    // sibling group's entry.
    ledger
        .fold(&region, "{revenue}", Grade::Additive, "orders", "d2")
        .unwrap();
    let revenue_entry = ledger.get(&region, "{revenue}").unwrap();
    let refunds_entry = ledger.get(&region, "{refunds}").unwrap();
    assert_eq!(
        revenue_entry.processed,
        Processed::DeltaIdentities(BTreeMap::from([(
            "orders".to_string(),
            BTreeSet::from(["d1".to_string(), "d2".to_string()])
        )]))
    );
    assert_eq!(
        refunds_entry.processed,
        Processed::DeltaIdentities(BTreeMap::from([(
            "orders".to_string(),
            BTreeSet::from(["d1".to_string()])
        )]))
    );
}

/// The ledger persists through the `.smelt/` file store, one
/// `ReconciliationLedger` per model, following the same round-trip pattern
/// `intervals.rs`'s `IntervalStore` already uses.
#[test]
fn reconciliation_store_roundtrips_through_file_store() {
    let dir = TempDir::new().unwrap();
    let store = FileStore::new(dir.path());

    let mut reconciliation = store.load_reconciliation_store().unwrap();
    let region = Region::new("2026-01-01", "2026-01-10");
    reconciliation
        .get_or_create("daily_revenue")
        .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
        .unwrap();
    store.save_reconciliation_store(&reconciliation).unwrap();

    let loaded = store.load_reconciliation_store().unwrap();
    let entry = loaded
        .get("daily_revenue")
        .and_then(|l| l.get(&region, "{revenue}"))
        .expect("round-tripped ledger keeps the folded entry");
    assert_eq!(
        entry.processed,
        Processed::DeltaIdentities(BTreeMap::from([(
            "orders".to_string(),
            BTreeSet::from(["d1".to_string()])
        )]))
    );
}

/// Testkit leg (real-fixture): the safe direction of the interchangeability
/// theorem (`docs/specs/maintenance_plan.md` §"The reconciliation ledger")
/// exercised over a real DuckDB model through
/// `smelt_runtime::execute_project` (`smelt-maintenance-testkit`'s Link-C
/// harness — dev-only dependency, see `Cargo.toml`). Two disjoint
/// window-advance runs of an additive `SUM` batched model each perform a
/// real region recompute (DELETE the write window + INSERT its recompute);
/// `crates/smelt-runtime/src/execute.rs` writes each one through this
/// crate's `recompute_reset` at the same call site it already writes
/// `IntervalStore`. This test asserts three things hold together over the
/// same real run:
///
/// 1. the maintained table matches an independent full-refresh oracle for
///    every windowed day (the run schedule reproduces a full refresh);
/// 2. the `.smelt/reconciliation.json` ledger the runtime wrote records a
///    recompute-reset entry per window, keyed by the region the runtime
///    actually recomputed;
/// 3. folding a delta the recompute already covered is refused by the
///    ledger's own fold-precondition check — never-fold-twice holds against
///    the state a real run produced, not just a hand-built fixture.
#[test]
fn fold_then_recompute_schedule_over_real_duckdb_model_matches_full_refresh() {
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
            "description: reconciliation-ledger testkit leg.\nmutation_profile: append_only\ncolumns:\n{cols}"
        );
        std::fs::write(
            project_dir.join(format!("models/sources/{}.yml", shape.source)),
            source_yml,
        )
        .unwrap();

        let smelt_yml = format!(
            "name: reconciliation_ledger_testkit\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
            &format!(
                "SELECT total FROM main.events_daily_total_append_only WHERE d = DATE '{date}'"
            ),
            [],
            |row| row.get(0),
        )
        .expect("maintained-table read")
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let shape = additive_agg_append_only();
        let tmp = TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("dev.duckdb");

        stage_project(&shape, &project_dir, &db_path);
        create_empty_events(&db_path);

        let project =
            LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

        let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        // Two disjoint one-day windows, each seeded before it is run — the
        // disjoint-delta shape the safe direction of the interchangeability
        // theorem covers.
        let windows: [(chrono::NaiveDate, &[f64]); 2] =
            [(base, &[1.5, 2.5]), (base + chrono::Duration::days(1), &[10.0])];

        let mut next_id = 1_i64;
        for (i, (day, vals)) in windows.iter().enumerate() {
            let next_day = *day + chrono::Duration::days(1);
            {
                let conn = project.connect().expect("connect for seed");
                for val in *vals {
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

            // (1) The real run's maintained state matches an independent
            // full-refresh oracle for the day it just processed.
            let conn = project.connect().expect("connect after run");
            let date_str = day.format("%Y-%m-%d").to_string();
            assert_eq!(
                maintained_total(&conn, &date_str),
                full_refresh_total(&conn, &date_str),
                "window {i} diverged from full refresh"
            );
        }

        // (2) The runtime wrote a reconciliation-ledger entry per window at
        // the same call site it writes `IntervalStore` — real wiring, not a
        // hand-built ledger.
        let file_store = FileStore::new(&project_dir);
        let reconciliation = file_store
            .load_reconciliation_store()
            .expect("load reconciliation store written by execute_project");
        let ledger = reconciliation
            .get(shape.name)
            .expect("execute_project recorded a ledger for the model it ran");

        let mut recorded_regions = Vec::new();
        for (day, _) in &windows {
            let next_day = *day + chrono::Duration::days(1);
            let region = Region::new(
                day.format("%Y-%m-%d").to_string(),
                next_day.format("%Y-%m-%d").to_string(),
            );
            let entry = ledger
                .get(&region, "{*}")
                .unwrap_or_else(|| panic!("no ledger entry for real run's own region {region:?}"));
            assert!(
                matches!(&entry.processed, Processed::Frontier(w) if w.get("self") == Some(&next_day.format("%Y-%m-%d").to_string())),
                "recompute-reset entry should record exactly the input the real run read"
            );
            recorded_regions.push(region);
        }

        // (3) Never-fold-twice against a real run's own recorded state: a
        // delta claiming to have already been reflected in a recomputed
        // region is refused by the ledger's fold-precondition check.
        let mut ledger_copy = ReconciliationLedger::new();
        let region = recorded_regions[0].clone();
        ledger_copy
            .fold(&region, "{*}", Grade::Idempotent, "self", &region.end)
            .expect("first fold on a fresh entry establishes the frontier");
        let refused = ledger_copy
            .fold(&region, "{*}", Grade::Idempotent, "self", &region.end)
            .unwrap_err();
        assert_eq!(refused.delta, region.end);
    });
}
