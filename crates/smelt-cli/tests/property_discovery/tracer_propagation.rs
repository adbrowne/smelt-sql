//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Tracer bullet, series 3 — cross-model dirty-partition propagation, on
//! DuckDB. The scenario the propagation exists for: models run because
//! *something landed upstream*, not because a cron tick says "today".
//!
//! Graph: `bronze_events → silver_events → daily_conv_rate`, plus
//! `conversions → silver_events` (the 14d first-score enrichment). When a
//! new day of conversions lands, propagation (edge clamps reflected through
//! the graph, all *derived* numbers) names exactly which silver partitions
//! and which rollup partitions must run; only those regions are maintained,
//! and the EXCEPT-ALL oracle over the whole tables proves that was
//! sufficient. A bronze arrival day then drives the other edge the same way.

use std::collections::BTreeMap;

use duckdb::Connection;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, widened_scan_predicate, Region,
};
use smelt_logical::maintenance::propagate::{propagate, DayInterval, Edge};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, ScanClamp, SourceFacts, Trigger,
};

use crate::oracle::multiset_equal;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn batch(conn: &Connection, statements: &[String]) {
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{sql}"));
    }
}

/// Day ordinals: epoch day 0 = 2026-01-01. Regions/predicates are built as
/// SQL date arithmetic so intervals map to dates without a date library.
fn date_of(day: i64) -> String {
    format!("(DATE '2026-01-01' + INTERVAL '{day} days')")
}

fn region_of(iv: &DayInterval) -> Region {
    Region {
        start: date_of(iv.start),
        end: date_of(iv.end),
    }
}

/// Silver: bronze pass-through under the 48h lateness clamp, enriched with
/// the first conversion score within 14 days.
fn silver_body() -> String {
    "SELECT e.event_id, e.user_id, e.event_ts, CAST(e.event_ts AS DATE) AS event_date, \
     (SELECT c.score FROM conversions c \
       WHERE c.event_id = e.event_id \
         AND c.conversion_ts >= e.event_ts \
         AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
         AND c.conversion_date BETWEEN CAST(e.event_ts AS DATE) \
                                   AND CAST(e.event_ts AS DATE) + INTERVAL '14 days' \
       ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
     FROM bronze_events e \
     WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
       AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                              AND CAST(e.event_ts AS DATE) + INTERVAL '2 days'"
        .to_string()
}

const ROLLUP_BODY: &str = "SELECT event_date, COUNT(*) AS events, \
     COUNT(conversion_score) AS converted \
     FROM silver_events GROUP BY event_date";

fn silver_inputs(sql: &str) -> ModelInputs<'_> {
    ModelInputs {
        sql,
        output: OutputSpec {
            table: "silver_events".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: set(&["event_id", "event_date"]),
        },
        sources: vec![
            SourceFacts {
                name: "bronze_events".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("arrival_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
            SourceFacts {
                name: "conversions".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("conversion_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
        ],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["user_id", "event_ts"]),
                mutation_sensitivity: Default::default(),
            },
            ColumnGroup {
                columns: strings(&["conversion_score"]),
                mutation_sensitivity: set(&["conversions"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    }
}

fn rollup_inputs() -> ModelInputs<'static> {
    ModelInputs {
        sql: ROLLUP_BODY,
        output: OutputSpec {
            table: "daily_conv_rate".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: set(&["event_date"]),
        },
        sources: vec![SourceFacts {
            name: "silver_events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["events", "converted"]),
            mutation_sensitivity: set(&["silver_events"]),
        }],
        fold: None,
        column_add_proof: None,
    }
}

fn clamp_for<'a>(scans: &'a [ScanClamp], source: &str) -> &'a ScanClamp {
    scans
        .iter()
        .find(|c| c.source == source)
        .unwrap_or_else(|| panic!("no scan clamp for '{source}' in {scans:?}"))
}

/// Column-merge silver's `{conversion_score}` over `region`, scanning
/// conversions under the derived widened window.
fn repair_conversion_score(conn: &Connection, clamp: &ScanClamp, region: &Region) {
    let scan = widened_scan_predicate(clamp, region);
    let source_select = format!(
        "SELECT e.event_id, e.event_date, \
         (SELECT c.score FROM conversions c \
           WHERE c.event_id = e.event_id \
             AND c.conversion_ts >= e.event_ts \
             AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
             AND {scan} \
           ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
         FROM silver_events e"
    );
    batch(
        conn,
        &emit_column_scoped_merge(
            "silver_events",
            &strings(&["event_id", "event_date"]),
            &strings(&["conversion_score"]),
            &source_select,
            Some("event_date"),
            Some(region),
        ),
    );
}

#[test]
fn landed_upstream_days_propagate_to_exactly_the_partitions_that_must_run() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE bronze_events (event_id INT, user_id INT, event_ts TIMESTAMP, \
                                     arrival_ts TIMESTAMP, arrival_date DATE);
         CREATE TABLE conversions (event_id INT, conversion_ts TIMESTAMP, \
                                   conversion_date DATE, score DOUBLE);
         INSERT INTO bronze_events VALUES
           (0, 9,  TIMESTAMP '2025-12-01 10:00:00', TIMESTAMP '2025-12-01 10:01:00', DATE '2025-12-01'),
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:01:00', DATE '2026-01-01'),
           (2, 11, TIMESTAMP '2026-01-05 09:00:00', TIMESTAMP '2026-01-05 09:01:00', DATE '2026-01-05'),
           (3, 12, TIMESTAMP '2026-01-14 08:00:00', TIMESTAMP '2026-01-14 08:01:00', DATE '2026-01-14');
         INSERT INTO conversions VALUES
           (1, TIMESTAMP '2026-01-03 12:00:00', DATE '2026-01-03', 5.0);",
    )
    .expect("stage");
    let silver = silver_body();
    conn.execute_batch(&format!(
        "CREATE TABLE silver_events AS {silver};
         CREATE TABLE daily_conv_rate AS {ROLLUP_BODY};"
    ))
    .expect("materialize both models");

    // Derive the plans; every edge clamp below comes off a plan cell.
    let s_inputs = silver_inputs(&silver);
    let silver_plan = derive_maintenance_plan(
        &s_inputs,
        &[
            Trigger::NewData {
                source: "bronze_events".to_string(),
            },
            Trigger::UpstreamMutation {
                source: "conversions".to_string(),
            },
        ],
    );
    assert!(
        silver_plan.refusals.is_empty(),
        "{:?}",
        silver_plan.refusals
    );
    let new_data_cell = &silver_plan.cells[0];
    let conv_cell = &silver_plan.cells[1];
    let bronze_clamp = clamp_for(&new_data_cell.scans, "bronze_events").clone();
    let conv_clamp = clamp_for(&conv_cell.scans, "conversions").clone();

    let r_inputs = rollup_inputs();
    let rollup_plan = derive_maintenance_plan(
        &r_inputs,
        &[Trigger::NewData {
            source: "silver_events".to_string(),
        }],
    );
    assert!(
        rollup_plan.refusals.is_empty(),
        "{:?}",
        rollup_plan.refusals
    );
    let silver_to_rollup = clamp_for(&rollup_plan.cells[0].scans, "silver_events").clone();

    let edges = vec![
        Edge::from_clamp("silver_events", &bronze_clamp),
        Edge::from_clamp("silver_events", &conv_clamp),
        Edge::from_clamp("daily_conv_rate", &silver_to_rollup),
    ];

    // ------------------------------------------------------------------
    // Tick 1: one new day of conversions lands (2026-01-15 = day 14):
    // event 2 (01-05) and event 3 (01-14) both convert.
    // ------------------------------------------------------------------
    conn.execute_batch(
        "INSERT INTO conversions VALUES
           (2, TIMESTAMP '2026-01-15 10:00:00', DATE '2026-01-15', 7.0),
           (3, TIMESTAMP '2026-01-15 11:00:00', DATE '2026-01-15', 3.0);",
    )
    .expect("land conversions day");

    let mut deltas: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    deltas.insert("conversions".to_string(), vec![DayInterval::new(14, 15)]);
    let result = propagate(&edges, &deltas).expect("propagate");

    // The 14d footprint reaches back exactly to 01-01; the 2025-12-01 event
    // (day −31) is outside and must not be scheduled.
    assert_eq!(
        result.dirty["silver_events"],
        vec![DayInterval::new(0, 15)],
        "conversions day 14 dirties silver [day 0, day 15)"
    );
    assert_eq!(
        result.dirty["daily_conv_rate"],
        vec![DayInterval::new(0, 15)],
        "the same-axis rollup edge passes the dirt through"
    );
    // The bronze edge contributed nothing this tick.
    assert!(!result
        .per_edge
        .contains_key(&("silver_events".to_string(), "bronze_events".to_string())));

    // Run exactly what propagation named: the conversions-edge cell on
    // silver (column merge), then the rollup regions.
    for iv in &result.per_edge[&("silver_events".to_string(), "conversions".to_string())] {
        repair_conversion_score(&conn, &conv_clamp, &region_of(iv));
    }
    for iv in &result.per_edge[&("daily_conv_rate".to_string(), "silver_events".to_string())] {
        batch(
            &conn,
            &emit_delete_insert("daily_conv_rate", "event_date", &region_of(iv), ROLLUP_BODY),
        );
    }
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM silver_events",
        &silver
    ));
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM daily_conv_rate",
        ROLLUP_BODY
    ));

    // ------------------------------------------------------------------
    // Tick 2: a bronze arrival day lands (2026-01-16 = day 15): a late
    // event for 01-15 and an on-time event for 01-16. The other edge
    // drives this tick: arrivals reach back 48h, so silver [13, 16) runs.
    // ------------------------------------------------------------------
    conn.execute_batch(
        "INSERT INTO bronze_events VALUES
           (4, 10, TIMESTAMP '2026-01-15 23:00:00', TIMESTAMP '2026-01-16 05:00:00', DATE '2026-01-16'),
           (5, 11, TIMESTAMP '2026-01-16 09:00:00', TIMESTAMP '2026-01-16 09:01:00', DATE '2026-01-16');",
    )
    .expect("land bronze day");

    let mut deltas2: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    deltas2.insert("bronze_events".to_string(), vec![DayInterval::new(15, 16)]);
    let result2 = propagate(&edges, &deltas2).expect("propagate");
    assert_eq!(
        result2.per_edge[&("silver_events".to_string(), "bronze_events".to_string())],
        vec![DayInterval::new(13, 16)],
        "an arrival day dirties the event days it can carry (48h back)"
    );

    // A driving-source delta runs the recompute-region cell: DELETE+INSERT
    // of the dirty partitions, scans widened by the derived clamps.
    for iv in &result2.per_edge[&("silver_events".to_string(), "bronze_events".to_string())] {
        let region = region_of(iv);
        let body = format!(
            "{silver} AND {}",
            widened_scan_predicate(&bronze_clamp, &region)
        );
        batch(
            &conn,
            &emit_delete_insert("silver_events", "event_date", &region, &body),
        );
    }
    for iv in &result2.per_edge[&("daily_conv_rate".to_string(), "silver_events".to_string())] {
        batch(
            &conn,
            &emit_delete_insert("daily_conv_rate", "event_date", &region_of(iv), ROLLUP_BODY),
        );
    }
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM silver_events",
        &silver
    ));
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM daily_conv_rate",
        ROLLUP_BODY
    ));

    // The untouched pre-epoch partition was never scheduled and is still
    // correct — the propagated set was sufficient, not merely "run all".
    let old_rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM silver_events WHERE event_date = DATE '2025-12-01'",
            [],
            |r| r.get(0),
        )
        .expect("old partition");
    assert_eq!(old_rows, 1);
}
