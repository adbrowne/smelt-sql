//! `smelt_db::model_source_clamps` — per-source clamp observability for
//! editor hover (`docs/specs/incremental_shapes.md` §"Observing the
//! per-source clamp", `docs/outcomes/20260815-partition-grain-residue/
//! phases/06-plan.md`).

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::workspace_ingest::ingest_loaded_workspace;
use smelt_logical::BoundResult;

const SMELT_YML: &str = r#"
name: model_source_clamps_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: table
"#;

fn source_clamps(
    files: &[(&str, &str)],
    model_file: &str,
) -> std::collections::BTreeMap<String, BoundResult> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::model_source_clamps(&db, ws, file)
}

/// A partition-grain model with a 3-day lookback on its upstream's own
/// `timeseries.partition_column` gets a `Bounded` verdict naming that
/// column and the derived margin.
#[test]
fn model_source_clamps_derives_upstream_bounds() {
    let raw_orders = "---\nmaterialization: table\ntimeseries:\n  event_time_column: order_ts\n  \
                       partition_column: order_ts\n  granularity: day\n---\n\
                       SELECT CAST(order_ts AS TIMESTAMP) AS order_ts, amount FROM \
                       (VALUES (TIMESTAMP '2026-01-01 00:00:00', 1.0)) AS t(order_ts, amount)\n";
    let recent_orders = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
                          timeseries:\n  event_time_column: order_date\n  partition_column: order_date\n  \
                          granularity: day\n---\n\
                          SELECT CAST(order_ts AS DATE) AS order_date, amount FROM smelt.raw_orders \
                          WHERE order_ts >= CURRENT_DATE - INTERVAL '3 day'\n";

    let clamps = source_clamps(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/raw_orders.sql", raw_orders),
            ("models/recent_orders.sql", recent_orders),
        ],
        "recent_orders",
    );

    let bound = clamps
        .get("raw_orders")
        .unwrap_or_else(|| panic!("no clamp for raw_orders in {clamps:?}"));
    match bound {
        BoundResult::Bounded {
            source_partition_col,
            before,
            after,
        } => {
            assert_eq!(source_partition_col, "order_ts");
            assert_eq!(before.0, 3 * 86400);
            assert_eq!(after.0, 0);
        }
        other => panic!("expected Bounded, got {other:?}"),
    }
}

/// A model with no `timeseries:` of its own (not partition-grain) gets an
/// empty clamp map — hover has nothing to show.
#[test]
fn model_source_clamps_empty_for_non_partition_grain_model() {
    let plain = "---\nmaterialization: view\n---\nSELECT 1 AS x\n";
    let clamps = source_clamps(
        &[("smelt.yml", SMELT_YML), ("models/plain.sql", plain)],
        "plain",
    );
    assert!(clamps.is_empty(), "expected empty map, got {clamps:?}");
}
