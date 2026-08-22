//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Tracer bullet, series 2 — one model evolving through five versions (see
//! `crates/smelt-logical/tests/maintenance_tracer_evolution.rs` for the
//! derivation-side assertions). This is the DuckDB leg: every maintenance op
//! is emitted from the derived plan, its scan window is built from the
//! *derived* clamp (`widened_scan_predicate`), and the result is proven
//! multiset-equal to a full refresh of the same version. A wrongly-derived
//! window (too narrow after the expression-delimitation fix, or too wide
//! before it) fails the oracle, not just an assertion.
//!
//! Lives in `smelt-runtime/tests/` (moved from the `smelt-cli` property-
//! discovery tree): it only calls `smelt_logical::maintenance::*` against a
//! raw in-memory DuckDB connection, with no dependency on `smelt-cli` or
//! `execute_project` — `smelt-runtime` already carries both `smelt-logical`
//! and `duckdb` as dependencies, so this needed no new plumbing
//! (`docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`).

mod oracle;

use std::collections::BTreeSet;

use duckdb::Connection;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, widened_scan_predicate, MaintenanceDialect,
    Region, StatementGroup,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, PartitionLocal, ScanClamp, SourceFacts,
    Trigger,
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
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
fn clamped(body: &str, col: &str, region: &Region) -> String {
    format!(
        "SELECT * FROM ({body}) WHERE {col} >= {start} AND {col} < {end}",
        start = region.start,
        end = region.end,
    )
}

fn day(d: &str) -> String {
    format!("DATE '{d}'")
}

fn stage(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE bronze_events (event_id INT, user_id INT, event_ts TIMESTAMP, \
                                     arrival_ts TIMESTAMP, arrival_date DATE, page TEXT);
         CREATE TABLE sessions (session_id INT, user_id INT, session_start_ts TIMESTAMP, \
                                session_end_ts TIMESTAMP, session_start_date DATE);
         CREATE TABLE conversions (event_id INT, conversion_ts TIMESTAMP, \
                                   conversion_date DATE, score DOUBLE);",
    )
    .expect("stage sources");
}

/// v2/v3 bodies over the executable tables; `scan` is the derived widened
/// arrival scan for an incremental run, or `TRUE` for a full refresh.
fn v2_body(scan: &str) -> String {
    format!(
        "SELECT event_id, user_id, event_ts, arrival_ts, \
         CAST(event_ts AS DATE) AS event_date, page \
         FROM bronze_events \
         WHERE arrival_ts < event_ts + INTERVAL '48 hours' \
           AND arrival_date BETWEEN CAST(event_ts AS DATE) \
                                AND CAST(event_ts AS DATE) + INTERVAL '2 days' \
           AND {scan}"
    )
}

fn v3_body(scan: &str) -> String {
    format!(
        "{} QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_ts) = 1",
        v2_body(scan)
    )
}

const V4_JOIN: &str = " LEFT JOIN sessions s \
       ON s.user_id = e.user_id \
      AND e.event_ts >= s.session_start_ts \
      AND e.event_ts < s.session_end_ts \
      AND s.session_start_date BETWEEN CAST(e.event_ts AS DATE) - INTERVAL '2 days' \
                                   AND CAST(e.event_ts AS DATE)";

fn v4_body() -> String {
    format!(
        "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, \
         CAST(e.event_ts AS DATE) AS event_date, e.page, s.session_id \
         FROM bronze_events e{V4_JOIN} \
         WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
           AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '2 days' \
         QUALIFY ROW_NUMBER() OVER (PARTITION BY e.event_id ORDER BY e.arrival_ts) = 1"
    )
}

const CONVERSION_SUBQUERY: &str = "(SELECT c.score FROM conversions c \
      WHERE c.event_id = e.event_id \
        AND c.conversion_ts >= e.event_ts \
        AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
        AND c.conversion_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '14 days' \
      ORDER BY c.conversion_ts LIMIT 1)";

fn v5_body() -> String {
    format!(
        "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, \
         CAST(e.event_ts AS DATE) AS event_date, e.page, s.session_id, \
         {CONVERSION_SUBQUERY} AS conversion_score \
         FROM bronze_events e{V4_JOIN} \
         WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
           AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '2 days' \
         QUALIFY ROW_NUMBER() OVER (PARTITION BY e.event_id ORDER BY e.arrival_ts) = 1"
    )
}

fn bronze() -> SourceFacts {
    SourceFacts {
        name: "bronze_events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("arrival_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn sessions_src() -> SourceFacts {
    SourceFacts {
        name: "sessions".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("session_start_date".to_string()),
        unique_key: strings(&["session_id"]),
        allow_full_scan: false,
    }
}

fn conversions_src() -> SourceFacts {
    SourceFacts {
        name: "conversions".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("conversion_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn output() -> OutputSpec {
    OutputSpec {
        table: "silver_events".to_string(),
        grain: Grain::Partition {
            partition_col: "event_date".to_string(),
        },
        skeleton_columns: set(&["event_id", "event_date"]),
    }
}

fn clamp_for<'a>(scans: &'a [ScanClamp], source: &str) -> &'a ScanClamp {
    scans
        .iter()
        .find(|c| c.source == source)
        .unwrap_or_else(|| panic!("no scan clamp for '{source}' in {scans:?}"))
}

// ---------------------------------------------------------------------------
// v2 — lateness clamp: incremental runs read only the derived arrival window;
// late-in-window events land, too-late events are dropped consistently.
// ---------------------------------------------------------------------------

#[test]
fn v2_incremental_with_derived_arrival_scan_equals_full_refresh() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn);
    conn.execute_batch(
        "CREATE TABLE silver_events (event_id INT, user_id INT, event_ts TIMESTAMP, \
                                     arrival_ts TIMESTAMP, event_date DATE, page TEXT);
         INSERT INTO bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:05:00', DATE '2026-01-01', '/a');",
    )
    .expect("stage silver + run-1 arrivals");

    // Derive the plan; the scan clamp is the number that sizes every read.
    let full = v2_body("TRUE");
    let inputs = ModelInputs {
        sql: &full,
        output: output(),
        sources: vec![bronze()],
        column_groups: vec![ColumnGroup {
            columns: strings(&["user_id", "event_ts", "arrival_ts", "page"]),
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    let cell = &plan.cells[0];
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    let clamp = clamp_for(&cell.scans, "bronze_events").clone();

    // Run 1: land [01-01, 01-02).
    let r1 = Region {
        start: day("2026-01-01"),
        end: day("2026-01-02"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "silver_events",
            "event_date",
            &r1,
            &clamped(
                &v2_body(&widened_scan_predicate(&clamp, &r1)),
                "event_date",
                &r1,
            ),
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &full));

    // Run 2: a late-but-in-window arrival for day 1, a new day-2 event, and
    // a too-late arrival (49h) that must be dropped by both paths.
    conn.execute_batch(
        "INSERT INTO bronze_events VALUES
           (2, 11, TIMESTAMP '2026-01-01 12:00:00', TIMESTAMP '2026-01-02 22:00:00', DATE '2026-01-02', '/b'),
           (3, 12, TIMESTAMP '2026-01-01 12:00:00', TIMESTAMP '2026-01-03 13:30:00', DATE '2026-01-03', '/c'),
           (4, 10, TIMESTAMP '2026-01-02 09:00:00', TIMESTAMP '2026-01-02 09:01:00', DATE '2026-01-02', '/d');",
    )
    .expect("run-2 arrivals");
    let r2 = Region {
        start: day("2026-01-01"),
        end: day("2026-01-03"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "silver_events",
            "event_date",
            &r2,
            &clamped(
                &v2_body(&widened_scan_predicate(&clamp, &r2)),
                "event_date",
                &r2,
            ),
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &full));
    let too_late: i64 = conn
        .query_row(
            "SELECT count(*) FROM silver_events WHERE event_id = 3",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(too_late, 0, "the 49h-late event must be excluded");
}

// ---------------------------------------------------------------------------
// v3 — dedup within the lateness window: a duplicate arriving in a later run
// resolves to the same canonical row a full refresh picks, because the
// derived scan (event window + 2d) always covers the whole dedup window.
// ---------------------------------------------------------------------------

#[test]
fn v3_dedup_is_stable_under_incremental_maintenance() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn);
    conn.execute_batch(
        "CREATE TABLE silver_events (event_id INT, user_id INT, event_ts TIMESTAMP, \
                                     arrival_ts TIMESTAMP, event_date DATE, page TEXT);
         INSERT INTO bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:05:00', DATE '2026-01-01', '/a');",
    )
    .expect("stage");

    let full = v3_body("TRUE");
    let inputs = ModelInputs {
        sql: &full,
        output: output(),
        sources: vec![bronze()],
        column_groups: vec![ColumnGroup {
            columns: strings(&["user_id", "event_ts", "arrival_ts", "page"]),
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    let clamp = clamp_for(&plan.cells[0].scans, "bronze_events").clone();

    let r1 = Region {
        start: day("2026-01-01"),
        end: day("2026-01-02"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "silver_events",
            "event_date",
            &r1,
            &clamped(
                &v3_body(&widened_scan_predicate(&clamp, &r1)),
                "event_date",
                &r1,
            ),
            MaintenanceDialect::DuckDb,
        ),
    );

    // A duplicate delivery of event 1 arrives the next day (within 48h) with
    // different payload; the canonical row (earliest arrival, page '/a')
    // must win in the re-run exactly as it does in a full refresh.
    conn.execute_batch(
        "INSERT INTO bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-02 09:00:00', DATE '2026-01-02', '/dup');",
    )
    .expect("duplicate delivery");
    batch_group(
        &conn,
        &emit_delete_insert(
            "silver_events",
            "event_date",
            &r1,
            &clamped(
                &v3_body(&widened_scan_predicate(&clamp, &r1)),
                "event_date",
                &r1,
            ),
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &full));
    let (rows, page): (i64, String) = conn
        .query_row(
            "SELECT count(*), min(page) FROM silver_events WHERE event_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("dedup check");
    assert_eq!(rows, 1, "exactly one canonical row per event_id");
    assert_eq!(page, "/a", "earliest arrival wins");
}

// ---------------------------------------------------------------------------
// v4 — introduce session_id: column-scoped MERGE per partition whose session
// scan is the derived 2-day lookback (a previous-day session must be found).
// ---------------------------------------------------------------------------

#[test]
fn v4_session_field_introduction_catches_up_with_the_derived_lookback() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn);
    conn.execute_batch(
        "INSERT INTO bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:05:00', DATE '2026-01-01', '/a'),
           (2, 11, TIMESTAMP '2026-01-01 12:00:00', TIMESTAMP '2026-01-01 12:01:00', DATE '2026-01-01', '/b'),
           (4, 10, TIMESTAMP '2026-01-02 09:00:00', TIMESTAMP '2026-01-02 09:01:00', DATE '2026-01-02', '/d');
         INSERT INTO sessions VALUES
           (100, 10, TIMESTAMP '2025-12-31 18:00:00', TIMESTAMP '2026-01-01 18:00:00', DATE '2025-12-31'),
           (200, 10, TIMESTAMP '2026-01-02 08:00:00', TIMESTAMP '2026-01-02 20:00:00', DATE '2026-01-02');",
    )
    .expect("stage");
    // Materialize v3, then the definition gains session_id (NULL = S = ∅).
    conn.execute_batch(&format!(
        "CREATE TABLE silver_events AS {};
         ALTER TABLE silver_events ADD COLUMN session_id INT;",
        v3_body("TRUE")
    ))
    .expect("materialize v3 + add column");

    let v4 = v4_body();
    let inputs = ModelInputs {
        sql: &v4,
        output: output(),
        sources: vec![bronze(), sessions_src()],
        column_groups: vec![ColumnGroup {
            columns: strings(&["session_id"]),
            mutation_sensitivity: set(&["sessions"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["session_id"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert!(cell.ledger_catch_up);
    let clamp = clamp_for(&cell.scans, "sessions").clone();

    for (start, end) in [("2026-01-01", "2026-01-02"), ("2026-01-02", "2026-01-03")] {
        let region = Region {
            start: day(start),
            end: day(end),
        };
        let scan = widened_scan_predicate(&clamp, &region);
        // The source projects the full target row — every v3 column carried
        // through unchanged from `silver_events` itself, `session_id`
        // re-derived — per the full-row-projection contract `UPDATE SET *`
        // relies on.
        let source_select = format!(
            "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, e.event_date, e.page, \
             s.session_id \
             FROM silver_events e \
             LEFT JOIN sessions s \
               ON s.user_id = e.user_id \
              AND e.event_ts >= s.session_start_ts \
              AND e.event_ts < s.session_end_ts \
              AND {scan}"
        );
        batch_group(
            &conn,
            &emit_column_scoped_merge(
                "silver_events",
                &strings(&["event_id", "event_date"]),
                &source_select,
                &[],
                MaintenanceDialect::DuckDb,
            ),
        );
    }
    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &v4));
    // The previous-day session is only found because the derived lookback is
    // 2 days — the clamp is doing the work, not a full sessions scan.
    let e1_session: i64 = conn
        .query_row(
            "SELECT session_id FROM silver_events WHERE event_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("e1 session");
    assert_eq!(e1_session, 100);
    let e2_session: Option<i64> = conn
        .query_row(
            "SELECT session_id FROM silver_events WHERE event_id = 2",
            [],
            |r| r.get(0),
        )
        .expect("e2 session");
    assert_eq!(e2_session, None, "no session for user 11");
}

// ---------------------------------------------------------------------------
// v5 — introduce conversion_score (first score within 14d), then repair it
// when late conversions arrive: the ongoing cell's write window is the
// derived clamp *reflected* (footprint), and a later second conversion for an
// already-scored event must not displace the first.
// ---------------------------------------------------------------------------

#[test]
fn v5_conversion_field_introduction_and_late_conversion_repair() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn);
    conn.execute_batch(
        "INSERT INTO bronze_events VALUES
           (1, 10, TIMESTAMP '2026-01-01 10:00:00', TIMESTAMP '2026-01-01 10:05:00', DATE '2026-01-01', '/a'),
           (4, 10, TIMESTAMP '2026-01-02 09:00:00', TIMESTAMP '2026-01-02 09:01:00', DATE '2026-01-02', '/d');
         INSERT INTO sessions VALUES
           (100, 10, TIMESTAMP '2025-12-31 18:00:00', TIMESTAMP '2026-01-01 18:00:00', DATE '2025-12-31');
         INSERT INTO conversions VALUES
           (1, TIMESTAMP '2026-01-03 09:00:00', DATE '2026-01-03', 5.0),
           (1, TIMESTAMP '2026-01-06 09:00:00', DATE '2026-01-06', 9.0);",
    )
    .expect("stage");
    conn.execute_batch(&format!(
        "CREATE TABLE silver_events AS {};
         ALTER TABLE silver_events ADD COLUMN conversion_score DOUBLE;",
        v4_body()
    ))
    .expect("materialize v4 + add column");

    let v5 = v5_body();
    let inputs = ModelInputs {
        sql: &v5,
        output: output(),
        sources: vec![bronze(), sessions_src(), conversions_src()],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["session_id"]),
                mutation_sensitivity: set(&["sessions"]),
                membership_sensitivity: BTreeSet::new(),
            },
            ColumnGroup {
                columns: strings(&["conversion_score"]),
                mutation_sensitivity: set(&["conversions"]),
                membership_sensitivity: BTreeSet::new(),
            },
        ],
        fold: None,
        old_columns: Vec::new(),
    };

    // Introduction: catch up the added column per partition with the derived
    // forward scan [P, P + 14d).
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["conversion_score"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let clamp = clamp_for(&plan.cells[0].scans, "conversions").clone();

    let merge_region = |conn: &Connection, region: &Region| {
        let scan = widened_scan_predicate(&clamp, region);
        // The source projects the full target row — every v4 column carried
        // through unchanged from `silver_events` itself, `conversion_score`
        // re-derived — per the full-row-projection contract `UPDATE SET *`
        // relies on.
        let source_select = format!(
            "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, e.event_date, e.page, \
             e.session_id, \
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
                &[],
                MaintenanceDialect::DuckDb,
            ),
        );
    };
    for (start, end) in [("2026-01-01", "2026-01-02"), ("2026-01-02", "2026-01-03")] {
        merge_region(
            &conn,
            &Region {
                start: day(start),
                end: day(end),
            },
        );
    }
    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &v5));

    // Ongoing: two late conversions arrive — event 4 gets its first score,
    // and event 1 gets a *later* second conversion that must not displace
    // its existing first (order-sensitive first-value, EX-35's shape). The
    // repair window is the derived clamp reflected: a conversion at t writes
    // events over [t − 14d, t], so the delta span [01-08, 01-11) repairs
    // event dates [01-08 − 14d, 01-11).
    conn.execute_batch(
        "INSERT INTO conversions VALUES
           (4, TIMESTAMP '2026-01-10 11:00:00', DATE '2026-01-10', 7.0),
           (1, TIMESTAMP '2026-01-08 08:00:00', DATE '2026-01-08', 1.0);",
    )
    .expect("late conversions");

    let mutation_plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "conversions".to_string(),
        }],
    );
    let mutation_cell = mutation_plan
        .cells
        .iter()
        .find(|c| c.group == "{conversion_score}")
        .expect("conversion_score mutation cell");
    let (fp_before, fp_after) = clamp_for(&mutation_cell.scans, "conversions").footprint();
    assert_eq!(fp_after.0, 0);
    let repair = Region {
        start: format!("DATE '2026-01-08' - INTERVAL '{} seconds'", fp_before.0),
        end: day("2026-01-11"),
    };
    merge_region(&conn, &repair);

    assert!(multiset_equal(&conn, "SELECT * FROM silver_events", &v5));
    let e1_score: f64 = conn
        .query_row(
            "SELECT conversion_score FROM silver_events WHERE event_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("e1 score");
    assert_eq!(e1_score, 5.0, "the first conversion keeps winning");
    let e4_score: f64 = conn
        .query_row(
            "SELECT conversion_score FROM silver_events WHERE event_id = 4",
            [],
            |r| r.get(0),
        )
        .expect("e4 score");
    assert_eq!(e4_score, 7.0, "the late conversion landed");
}
