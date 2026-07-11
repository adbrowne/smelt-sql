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
//!
//! Lives in `smelt-runtime/tests/` (moved from the `smelt-cli` property-
//! discovery tree): it only calls `smelt_logical::maintenance::*` against a
//! raw in-memory DuckDB connection, with no dependency on `smelt-cli` or
//! `execute_project` — `smelt-runtime` already carries both `smelt-logical`
//! and `duckdb` as dependencies, so this needed no new plumbing
//! (`docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`).

mod oracle;

use std::collections::BTreeMap;

use duckdb::Connection;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, widened_scan_predicate, MaintenanceDialect,
    Region, StatementGroup,
};
use smelt_logical::maintenance::propagate::{propagate, DayInterval, Edge};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, ScanClamp, SourceFacts, Trigger,
};

use oracle::multiset_equal;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn batch_group(conn: &Connection, group: &StatementGroup) {
    for stmt in &group.statements {
        conn.execute_batch(&stmt.sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{}", stmt.sql));
    }
}

/// Test-only stand-in for the runtime's output-clamp injection
/// (`smelt-runtime/src/transformer.rs`): the single-owner emitter contract
/// requires the caller to fold the region predicate into the body it hands
/// `emit_delete_insert` — the emitter itself no longer adds one
/// (`docs/specs/maintenance_plan.md` §"Statement emission (single owner)").
fn clamped(body: &str, col: &str, region: &Region) -> String {
    format!(
        "SELECT * FROM ({body}) WHERE {col} >= {start} AND {col} < {end}",
        start = region.start,
        end = region.end,
    )
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
    // The source projects the full target row (event_id, user_id, event_ts,
    // event_date, conversion_score) — the caller-side full-row-projection
    // contract `UPDATE SET *` relies on — carrying every column but
    // `conversion_score` through unchanged from `silver_events` itself.
    let source_select = format!(
        "SELECT e.event_id, e.user_id, e.event_ts, e.event_date, \
         (SELECT c.score FROM conversions c \
           WHERE c.event_id = e.event_id \
             AND c.conversion_ts >= e.event_ts \
             AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
             AND {scan} \
           ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
         FROM silver_events e"
    );
    batch_group(
        conn,
        &emit_column_scoped_merge(
            "silver_events",
            &strings(&["event_id", "event_date"]),
            &source_select,
            MaintenanceDialect::DuckDb,
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
        let region = region_of(iv);
        batch_group(
            &conn,
            &emit_delete_insert(
                "daily_conv_rate",
                "event_date",
                &region,
                &clamped(ROLLUP_BODY, "event_date", &region),
                MaintenanceDialect::DuckDb,
            ),
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
        batch_group(
            &conn,
            &emit_delete_insert(
                "silver_events",
                "event_date",
                &region,
                &clamped(&body, "event_date", &region),
                MaintenanceDialect::DuckDb,
            ),
        );
    }
    for iv in &result2.per_edge[&("daily_conv_rate".to_string(), "silver_events".to_string())] {
        let region = region_of(iv);
        batch_group(
            &conn,
            &emit_delete_insert(
                "daily_conv_rate",
                "event_date",
                &region,
                &clamped(ROLLUP_BODY, "event_date", &region),
                MaintenanceDialect::DuckDb,
            ),
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

// ---------------------------------------------------------------------------
// Backward resolution on DuckDB (S6/S7 of 10-dependency-propagation.md):
// "build daily_conv_rate for [Jan 5, Jan 8) including upstreams". The
// universe lives in schema `uni`; the sandbox is staged with EXACTLY the
// resolved per-source slices, built bottom-up in build_order, and the target
// period must equal what a build over complete history produces. A sentinel
// event converts AFTER the period's edge (day 17), so an under-widened
// conversions slice yields a NULL score — the naive build is run first and
// must diverge, proving the widening is load-bearing.
// ---------------------------------------------------------------------------

use smelt_logical::maintenance::propagate::required_inputs;

/// Silver over prefixed source tables: 48h lateness clamp, session
/// enrichment (48h max length), first conversion score within 14d.
fn sb_silver_body(p: &str) -> String {
    format!(
        "SELECT e.event_id, e.user_id, e.event_ts, CAST(e.event_ts AS DATE) AS event_date, \
         s.session_id, \
         (SELECT c.score FROM {p}conversions c \
           WHERE c.event_id = e.event_id \
             AND c.conversion_ts >= e.event_ts \
             AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
             AND c.conversion_date BETWEEN CAST(e.event_ts AS DATE) \
                                       AND CAST(e.event_ts AS DATE) + INTERVAL '14 days' \
           ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
         FROM {p}bronze_events e \
         LEFT JOIN {p}sessions s \
           ON s.user_id = e.user_id \
          AND e.event_ts >= s.session_start_ts \
          AND e.event_ts < s.session_end_ts \
          AND s.session_start_date BETWEEN CAST(e.event_ts AS DATE) - INTERVAL '2 days' \
                                       AND CAST(e.event_ts AS DATE) \
         WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
           AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '2 days'"
    )
}

fn sb_rollup_body(silver_rel: &str) -> String {
    format!(
        "SELECT event_date, COUNT(*) AS events, COUNT(conversion_score) AS converted, \
         COUNT(session_id) AS in_session FROM {silver_rel} GROUP BY event_date"
    )
}

fn sb_silver_inputs(sql: &str) -> ModelInputs<'_> {
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
                name: "sessions".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("session_start_date".to_string()),
                unique_key: strings(&["session_id"]),
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
                columns: strings(&["session_id"]),
                mutation_sensitivity: set(&["sessions"]),
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

/// Stage the sandbox source slices (CTAS from `uni`), build the models
/// bottom-up over `period` via the plan's recompute technique, and return
/// nothing — assertions live with the caller. `conversions_slice` is the
/// experiment variable (naive vs resolved).
fn build_sandbox(
    conn: &Connection,
    required: &BTreeMap<String, Vec<DayInterval>>,
    conversions_slice: &DayInterval,
    period: &DayInterval,
) {
    let slice_pred = |col: &str, iv: &DayInterval| {
        format!(
            "{col} >= {} AND {col} < {}",
            date_of(iv.start),
            date_of(iv.end)
        )
    };
    let bronze_iv = required["bronze_events"][0];
    let sessions_iv = required["sessions"][0];
    conn.execute_batch(&format!(
        "CREATE TABLE bronze_events AS SELECT * FROM uni.bronze_events WHERE {};
         CREATE TABLE sessions AS SELECT * FROM uni.sessions WHERE {};
         CREATE TABLE conversions AS SELECT * FROM uni.conversions WHERE {};",
        slice_pred("arrival_date", &bronze_iv),
        slice_pred("session_start_date", &sessions_iv),
        slice_pred("conversion_date", conversions_slice),
    ))
    .expect("stage sandbox slices");

    // Build in build_order (silver_events, then daily_conv_rate), each via
    // the plan's recompute-region technique over the requested period.
    let silver = sb_silver_body("");
    conn.execute_batch(&format!(
        "CREATE TABLE silver_events AS SELECT * FROM ({silver}) WHERE FALSE;"
    ))
    .expect("empty silver");
    let silver_region = region_of(period);
    batch_group(
        conn,
        &emit_delete_insert(
            "silver_events",
            "event_date",
            &silver_region,
            &clamped(&silver, "event_date", &silver_region),
            MaintenanceDialect::DuckDb,
        ),
    );
    let rollup = sb_rollup_body("silver_events");
    conn.execute_batch(&format!(
        "CREATE TABLE daily_conv_rate AS SELECT * FROM ({rollup}) WHERE FALSE;"
    ))
    .expect("empty rollup");
    let rollup_region = region_of(period);
    batch_group(
        conn,
        &emit_delete_insert(
            "daily_conv_rate",
            "event_date",
            &rollup_region,
            &clamped(&rollup, "event_date", &rollup_region),
            MaintenanceDialect::DuckDb,
        ),
    );
}

fn drop_sandbox(conn: &Connection) {
    conn.execute_batch(
        "DROP TABLE daily_conv_rate; DROP TABLE silver_events; \
         DROP TABLE conversions; DROP TABLE sessions; DROP TABLE bronze_events;",
    )
    .expect("drop sandbox");
}

#[test]
fn bounded_period_build_including_upstreams_matches_the_full_universe() {
    let conn = Connection::open_in_memory().expect("duckdb");
    // The universe: complete history, schema `uni`. Epoch day 0 = 2026-01-01.
    conn.execute_batch(
        "CREATE SCHEMA uni;
         CREATE TABLE uni.bronze_events (event_id INT, user_id INT, event_ts TIMESTAMP, \
                                         arrival_ts TIMESTAMP, arrival_date DATE);
         CREATE TABLE uni.sessions (session_id INT, user_id INT, session_start_ts TIMESTAMP, \
                                    session_end_ts TIMESTAMP, session_start_date DATE);
         CREATE TABLE uni.conversions (event_id INT, conversion_ts TIMESTAMP, \
                                       conversion_date DATE, score DOUBLE);
         INSERT INTO uni.bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:01:00', DATE '2026-01-01'),
           (3, 10, TIMESTAMP '2026-01-05 08:00:00', TIMESTAMP '2026-01-05 08:01:00', DATE '2026-01-05'),
           (2, 11, TIMESTAMP '2026-01-06 09:00:00', TIMESTAMP '2026-01-06 09:01:00', DATE '2026-01-06'),
           (6, 12, TIMESTAMP '2026-01-05 23:00:00', TIMESTAMP '2026-01-07 04:00:00', DATE '2026-01-07'),
           (4, 12, TIMESTAMP '2026-01-07 12:00:00', TIMESTAMP '2026-01-07 12:01:00', DATE '2026-01-07'),
           (7, 11, TIMESTAMP '2026-01-07 20:00:00', TIMESTAMP '2026-01-09 10:00:00', DATE '2026-01-09'),
           (5, 10, TIMESTAMP '2026-01-09 11:00:00', TIMESTAMP '2026-01-09 11:01:00', DATE '2026-01-09');
         INSERT INTO uni.sessions VALUES
           (100, 10, TIMESTAMP '2026-01-03 22:00:00', TIMESTAMP '2026-01-05 10:00:00', DATE '2026-01-03'),
           (200, 11, TIMESTAMP '2026-01-06 07:00:00', TIMESTAMP '2026-01-06 20:00:00', DATE '2026-01-06');
         INSERT INTO uni.conversions VALUES
           (3, TIMESTAMP '2026-01-06 15:00:00', DATE '2026-01-06', 2.0),
           (2, TIMESTAMP '2026-01-18 10:00:00', DATE '2026-01-18', 4.5),
           (3, TIMESTAMP '2026-01-17 09:00:00', DATE '2026-01-17', 8.8),
           (1, TIMESTAMP '2026-01-02 08:00:00', DATE '2026-01-02', 9.9);",
    )
    .expect("stage universe");

    // Derive the plans; the edges (and therefore the backward widening) are
    // the derived clamps, never hand-typed.
    let silver_sql = sb_silver_body("");
    let s_inputs = sb_silver_inputs(&silver_sql);
    let silver_plan = derive_maintenance_plan(
        &s_inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    assert!(
        silver_plan.refusals.is_empty(),
        "{:?}",
        silver_plan.refusals
    );
    let scans = &silver_plan.cells[0].scans;
    let rollup_sql = sb_rollup_body("silver_events");
    let mut r_inputs = rollup_inputs();
    r_inputs.sql = &rollup_sql;
    let rollup_plan = derive_maintenance_plan(
        &r_inputs,
        &[Trigger::NewData {
            source: "silver_events".to_string(),
        }],
    );
    let edges = vec![
        Edge::from_clamp("silver_events", clamp_for(scans, "bronze_events")),
        Edge::from_clamp("silver_events", clamp_for(scans, "sessions")),
        Edge::from_clamp("silver_events", clamp_for(scans, "conversions")),
        Edge::from_clamp(
            "daily_conv_rate",
            clamp_for(&rollup_plan.cells[0].scans, "silver_events"),
        ),
    ];

    // Backward resolution: build daily_conv_rate for [Jan 5, Jan 8) = [4, 7).
    let period = DayInterval::new(4, 7);
    let resolved = required_inputs(&edges, "daily_conv_rate", period).expect("resolve");
    assert_eq!(
        resolved.required["silver_events"],
        vec![DayInterval::new(4, 7)]
    );
    assert_eq!(
        resolved.required["bronze_events"],
        vec![DayInterval::new(4, 9)],
        "arrivals widen forward by the 48h lateness window"
    );
    assert_eq!(
        resolved.required["conversions"],
        vec![DayInterval::new(4, 21)],
        "conversions widen forward by the 14d attribution window"
    );
    assert_eq!(
        resolved.required["sessions"],
        vec![DayInterval::new(2, 7)],
        "sessions widen backward by the 48h max session length"
    );
    assert_eq!(
        resolved.build_order,
        vec!["silver_events".to_string(), "daily_conv_rate".to_string()]
    );

    // Universe baselines, restricted to the period.
    let uni_silver = sb_silver_body("uni.");
    let period_pred = format!(
        "event_date >= {} AND event_date < {}",
        date_of(period.start),
        date_of(period.end)
    );
    let silver_baseline = format!("SELECT * FROM ({uni_silver}) WHERE {period_pred}");
    let rollup_baseline = format!(
        "SELECT * FROM ({}) WHERE {period_pred}",
        sb_rollup_body(&format!("({uni_silver})"))
    );

    // --- Negative control: an under-widened conversions slice (the naive
    // [4, 7) guess) builds without error but silently diverges — the
    // sentinel event (id 2, converting on day 17) loses its score.
    build_sandbox(&conn, &resolved.required, &DayInterval::new(4, 7), &period);
    let naive_sentinel: Option<f64> = conn
        .query_row(
            "SELECT conversion_score FROM silver_events WHERE event_id = 2",
            [],
            |r| r.get(0),
        )
        .expect("naive sentinel");
    assert_eq!(
        naive_sentinel, None,
        "under-widened conversions slice must lose the late conversion"
    );
    assert!(
        !multiset_equal(&conn, "SELECT * FROM silver_events", &silver_baseline),
        "the naive build must diverge from the full-universe baseline"
    );
    drop_sandbox(&conn);

    // --- The resolved build: exact slices, bottom-up, equal to the
    // full-universe computation over the period.
    build_sandbox(
        &conn,
        &resolved.required,
        &resolved.required["conversions"][0],
        &period,
    );
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM silver_events",
        &silver_baseline
    ));
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM daily_conv_rate",
        &rollup_baseline
    ));
    // The sentinel proves the forward widening is load-bearing…
    let sentinel: f64 = conn
        .query_row(
            "SELECT conversion_score FROM silver_events WHERE event_id = 2",
            [],
            |r| r.get(0),
        )
        .expect("sentinel");
    assert_eq!(sentinel, 4.5);
    // …the boundary session proves the backward widening is (session 100
    // started on day 2, the first required sessions day)…
    let boundary_session: i64 = conn
        .query_row(
            "SELECT session_id FROM silver_events WHERE event_id = 3",
            [],
            |r| r.get(0),
        )
        .expect("boundary session");
    assert_eq!(boundary_session, 100);
    // …and the late arrival proves the bronze widening is (event 7 landed
    // on day 8, the last required arrivals day).
    let late_arrival_rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM silver_events WHERE event_id = 7",
            [],
            |r| r.get(0),
        )
        .expect("late arrival");
    assert_eq!(late_arrival_rows, 1);
}

// ---------------------------------------------------------------------------
// Granularity mapping on DuckDB (S10 of 10-dependency-propagation.md §7):
// a daily silver model feeds a monthly reporting model. A dirty day dirties
// its whole containing month; only that month is rebuilt, and a sibling
// month is never scheduled yet stays correct.
// ---------------------------------------------------------------------------

use smelt_logical::maintenance::propagate::{day_ordinal, ordinal_to_iso, PartitionGrain};

#[test]
fn a_dirty_day_rebuilds_exactly_its_containing_month_downstream() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE silver_events (event_id INT, event_date DATE, amount DOUBLE);
         INSERT INTO silver_events VALUES
           (1, DATE '2026-01-05', 5.0),
           (2, DATE '2026-01-17', 7.0),
           (3, DATE '2026-02-03', 9.0);",
    )
    .expect("stage silver");
    let monthly_body = "SELECT date_trunc('month', event_date) AS report_month, \
                        COUNT(*) AS events, SUM(amount) AS revenue \
                        FROM silver_events GROUP BY 1";
    conn.execute_batch(&format!("CREATE TABLE monthly_report AS {monthly_body};"))
        .expect("materialize monthly");

    // The edge: same clock axis, coarser downstream grain. The (0,0) clamp
    // is the derived same-axis clamp; the Month grain is declared on the
    // edge (deriving grain from the date_trunc projection is future
    // classifier work — 10-dependency-propagation.md §7).
    let e = Edge {
        upstream: "silver_events".to_string(),
        downstream: "monthly_report".to_string(),
        before_days: 0,
        after_days: 0,
        upstream_grain: PartitionGrain::Day,
        downstream_grain: PartitionGrain::Month,
    };

    // A January day changes upstream (a late-arriving event lands).
    conn.execute_batch("INSERT INTO silver_events VALUES (4, DATE '2026-01-17', 11.0);")
        .expect("land new silver row");
    let d17 = day_ordinal(2026, 1, 17);
    let mut deltas: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    deltas.insert(
        "silver_events".to_string(),
        vec![DayInterval::new(d17, d17 + 1)],
    );
    let result = propagate(&[e], &deltas).expect("propagate");

    let jan = DayInterval::new(day_ordinal(2026, 1, 1), day_ordinal(2026, 2, 1));
    assert_eq!(
        result.dirty["monthly_report"],
        vec![jan],
        "the dirty day dirties its whole containing month — and only that month"
    );

    // Rebuild exactly the propagated month partition.
    for iv in &result.dirty["monthly_report"] {
        let region = Region {
            start: format!("DATE '{}'", ordinal_to_iso(iv.start)),
            end: format!("DATE '{}'", ordinal_to_iso(iv.end)),
        };
        batch_group(
            &conn,
            &emit_delete_insert(
                "monthly_report",
                "report_month",
                &region,
                &clamped(monthly_body, "report_month", &region),
                MaintenanceDialect::DuckDb,
            ),
        );
    }
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM monthly_report",
        monthly_body
    ));
    // February was never scheduled and is still correct (one event, 9.0).
    let (feb_events, feb_revenue): (i64, f64) = conn
        .query_row(
            "SELECT events, revenue FROM monthly_report WHERE report_month = DATE '2026-02-01'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("feb row");
    assert_eq!((feb_events, feb_revenue), (1, 9.0));
}
