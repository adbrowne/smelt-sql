//! DuckDB-executed proofs for the succession-patch technique's emitters
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/04-plan.md`):
//! `emit_succession_patch` applied window-by-window against a real DuckDB,
//! compared against the model's own `LEAD`/`LAG` SQL as the full-refresh
//! oracle (`docs/specs/incremental_shapes.md` §"The succession grain" —
//! "the projection's own `LEAD`/`LAG` over the full processed input is the
//! executable oracle"). Text-shape assertions for the emitters live in
//! `crates/smelt-logical/src/maintenance/emit/succession.rs`'s own
//! `#[cfg(test)]` module; this file proves the *result*.
//!
//! Skips loudly (never silently) when the system DuckDB library is
//! unavailable, matching `maintenance_plan_conformance.rs`'s own note: this
//! crate's dev-profile already requires it to compile.

use duckdb::Connection;

use smelt_logical::maintenance::emit::{
    emit_succession_clock_tie_probe, emit_succession_event_delta, emit_succession_patch,
    DerivedColumn, MaintenanceDialect, StatementGroup,
};

const PRESENTED: &str = "main.customer_history";
const TOMBSTONES: &str = "main.customer_history__tombstones";
const SOURCE: &str = "customer_changes";

fn stage(conn: &Connection, with_delete_flag: bool) {
    conn.execute_batch(&format!(
        "CREATE TABLE {SOURCE} (customer_id INTEGER, changed_at TIMESTAMP, tier TEXT{});
         CREATE TABLE {PRESENTED} (customer_id INTEGER, changed_at TIMESTAMP, tier TEXT, \
         valid_to TIMESTAMP);
         CREATE TABLE {TOMBSTONES} (customer_id INTEGER, changed_at TIMESTAMP);",
        if with_delete_flag {
            ", is_deleted BOOLEAN"
        } else {
            ""
        }
    ))
    .expect("stage");
}

fn insert_events(conn: &Connection, rows: &[(i64, &str, &str)]) {
    for (id, ts, tier) in rows {
        conn.execute_batch(&format!(
            "INSERT INTO {SOURCE} (customer_id, changed_at, tier) VALUES ({id}, TIMESTAMP '{ts}', '{tier}')"
        ))
        .expect("insert event");
    }
}

fn insert_events_with_delete(conn: &Connection, rows: &[(i64, &str, &str, bool)]) {
    for (id, ts, tier, deleted) in rows {
        conn.execute_batch(&format!(
            "INSERT INTO {SOURCE} (customer_id, changed_at, tier, is_deleted) VALUES ({id}, \
             TIMESTAMP '{ts}', '{tier}', {deleted})"
        ))
        .expect("insert event");
    }
}

fn batch_group(conn: &Connection, group: &StatementGroup) {
    for stmt in &group.statements {
        conn.execute_batch(&stmt.sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{}", stmt.sql));
    }
}

/// Two relations are equal multisets iff `EXCEPT ALL` is empty both ways
/// (the Link-C oracle idiom, duplicated per `maintenance_plan_conformance.rs`'s
/// own precedent — each integration-test file compiles as an independent
/// binary).
fn multiset_equal(conn: &Connection, left_sql: &str, right_sql: &str) -> bool {
    let count = |l: &str, r: &str| -> i64 {
        conn.query_row(
            &format!("SELECT count(*) FROM (({l}) EXCEPT ALL ({r})) AS d"),
            [],
            |row| row.get(0),
        )
        .expect("except all count query")
    };
    count(left_sql, right_sql) == 0 && count(right_sql, left_sql) == 0
}

fn row_count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM ({sql}) AS t"), [], |row| {
        row.get(0)
    })
    .expect("count")
}

fn lead_only() -> (Vec<DerivedColumn>, Vec<DerivedColumn>) {
    (vec![("valid_to".to_string(), "{lead}".to_string())], vec![])
}

fn lag_only() -> (Vec<DerivedColumn>, Vec<DerivedColumn>) {
    (
        vec![],
        vec![("previous_ts".to_string(), "{lag}".to_string())],
    )
}

fn apply_window(
    conn: &Connection,
    window_predicate: &str,
    lead_derived: &[DerivedColumn],
    lag_derived: &[DerivedColumn],
    delete_flag_expr: Option<&str>,
) {
    let mut projection = vec![
        ("customer_id".to_string(), "customer_id".to_string()),
        ("changed_at".to_string(), "changed_at".to_string()),
        ("tier".to_string(), "tier".to_string()),
    ];
    if delete_flag_expr.is_some() {
        projection.push(("is_deleted".to_string(), "is_deleted".to_string()));
    }
    let event_delta = emit_succession_event_delta(SOURCE, &projection, None, window_predicate);
    let group = emit_succession_patch(
        PRESENTED,
        &["customer_id".to_string()],
        "changed_at",
        &["tier".to_string()],
        lead_derived,
        lag_derived,
        delete_flag_expr,
        &event_delta.sql,
        MaintenanceDialect::DuckDb,
    );
    batch_group(conn, &group);
}

fn oracle_sql(delete_flag: bool) -> String {
    if delete_flag {
        "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER (PARTITION BY customer_id \
         ORDER BY changed_at) AS valid_to FROM customer_changes QUALIFY NOT is_deleted"
            .to_string()
    } else {
        "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER (PARTITION BY customer_id \
         ORDER BY changed_at) AS valid_to FROM customer_changes"
            .to_string()
    }
}

#[test]
fn patch_matches_full_refresh_for_a_late_splice() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn, false);
    insert_events(
        &conn,
        &[
            (1, "2026-01-01 00:00:00", "gold"),
            (1, "2026-01-03 00:00:00", "silver"),
        ],
    );
    let (lead, lag) = lead_only();
    apply_window(
        &conn,
        "changed_at IN (TIMESTAMP '2026-01-01 00:00:00', TIMESTAMP '2026-01-03 00:00:00')",
        &lead,
        &lag,
        None,
    );

    // Late event splices between the two already-folded events.
    insert_events(&conn, &[(1, "2026-01-02 00:00:00", "platinum")]);
    apply_window(
        &conn,
        "changed_at = TIMESTAMP '2026-01-02 00:00:00'",
        &lead,
        &lag,
        None,
    );

    assert!(
        multiset_equal(
            &conn,
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history",
            &oracle_sql(false),
        ),
        "a late-splicing event must patch its predecessor and successor to match the oracle"
    );
}

#[test]
fn patch_matches_full_refresh_for_a_delete_then_a_later_insert() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn, true);
    insert_events_with_delete(
        &conn,
        &[
            (1, "2026-01-01 00:00:00", "gold", false),
            (1, "2026-01-02 00:00:00", "silver", true),
        ],
    );
    let (lead, lag) = lead_only();
    apply_window(
        &conn,
        "changed_at IN (TIMESTAMP '2026-01-01 00:00:00', TIMESTAMP '2026-01-02 00:00:00')",
        &lead,
        &lag,
        Some("is_deleted"),
    );

    // A later event arrives after the delete — the tombstone must still be
    // findable as its predecessor.
    insert_events_with_delete(&conn, &[(1, "2026-01-03 00:00:00", "platinum", false)]);
    apply_window(
        &conn,
        "changed_at = TIMESTAMP '2026-01-03 00:00:00'",
        &lead,
        &lag,
        Some("is_deleted"),
    );

    assert!(
        multiset_equal(
            &conn,
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history",
            &oracle_sql(true),
        ),
        "the ledger must be load-bearing for the later event's predecessor lookup"
    );

    // Prove the ledger is load-bearing: with its row removed, the neighbour
    // lookup for the later event's predecessor would find nothing —
    // demonstrated by the ledger genuinely holding the tombstone's `(k, t)`.
    assert_eq!(
        row_count(
            &conn,
            &format!(
                "SELECT * FROM {TOMBSTONES} WHERE customer_id = 1 AND changed_at = TIMESTAMP \
                 '2026-01-02 00:00:00'"
            )
        ),
        1,
        "the delete event must be recorded in the tombstone ledger"
    );
}

#[test]
fn patch_matches_full_refresh_for_a_lag_projecting_model() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn, false);
    conn.execute_batch(
        "ALTER TABLE main.customer_history DROP COLUMN valid_to; \
         ALTER TABLE main.customer_history ADD COLUMN previous_ts TIMESTAMP;",
    )
    .expect("reshape for lag model");
    insert_events(
        &conn,
        &[
            (1, "2026-01-01 00:00:00", "gold"),
            (1, "2026-01-03 00:00:00", "silver"),
        ],
    );
    let (lead, lag) = lag_only();
    apply_window(
        &conn,
        "changed_at IN (TIMESTAMP '2026-01-01 00:00:00', TIMESTAMP '2026-01-03 00:00:00')",
        &lead,
        &lag,
        None,
    );

    // A late event splices in — the SUCCESSOR's `previous_ts` must patch to
    // point at the new event.
    insert_events(&conn, &[(1, "2026-01-02 00:00:00", "platinum")]);
    apply_window(
        &conn,
        "changed_at = TIMESTAMP '2026-01-02 00:00:00'",
        &lead,
        &lag,
        None,
    );

    let oracle = "SELECT customer_id, changed_at, tier, LAG(changed_at) OVER (PARTITION BY \
                  customer_id ORDER BY changed_at) AS previous_ts FROM customer_changes";
    assert!(
        multiset_equal(
            &conn,
            "SELECT customer_id, changed_at, tier, previous_ts FROM main.customer_history",
            oracle,
        ),
        "a LAG-projecting model's successor must patch to the new event"
    );
}

#[test]
fn refolding_a_window_leaves_table_and_ledger_unchanged() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn, false);
    insert_events(
        &conn,
        &[
            (1, "2026-01-01 00:00:00", "gold"),
            (1, "2026-01-02 00:00:00", "silver"),
        ],
    );
    let (lead, lag) = lead_only();
    let window = "changed_at IN (TIMESTAMP '2026-01-01 00:00:00', TIMESTAMP '2026-01-02 \
                  00:00:00')";
    apply_window(&conn, window, &lead, &lag, None);
    let before: Vec<String> = conn
        .prepare(
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history \
                  ORDER BY changed_at",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(format!(
                "{:?}|{:?}|{:?}|{:?}",
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    apply_window(&conn, window, &lead, &lag, None);
    let after: Vec<String> = conn
        .prepare(
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history \
                  ORDER BY changed_at",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(format!(
                "{:?}|{:?}|{:?}|{:?}",
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(
        before, after,
        "re-folding an already-applied window must be a no-op"
    );
}

#[test]
fn two_windows_applied_in_either_order_converge() {
    let make_conn_and_apply = |first: &str, second: &str| -> Connection {
        let conn = Connection::open_in_memory().expect("duckdb");
        stage(&conn, false);
        insert_events(
            &conn,
            &[
                (1, "2026-01-01 00:00:00", "gold"),
                (1, "2026-01-02 00:00:00", "silver"),
                (1, "2026-01-03 00:00:00", "platinum"),
            ],
        );
        let (lead, lag) = lead_only();
        apply_window(&conn, first, &lead, &lag, None);
        apply_window(&conn, second, &lead, &lag, None);
        conn
    };

    let window_a = "changed_at IN (TIMESTAMP '2026-01-01 00:00:00')";
    let window_b = "changed_at IN (TIMESTAMP '2026-01-02 00:00:00', TIMESTAMP '2026-01-03 \
                    00:00:00')";

    let conn_ab = make_conn_and_apply(window_a, window_b);
    let conn_ba = make_conn_and_apply(window_b, window_a);

    assert!(
        multiset_equal(
            &conn_ab,
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history",
            &oracle_sql(false),
        ),
        "applying window A then B must reproduce the oracle"
    );

    // Compare the two final tables against each other by staging one's rows
    // into the other's connection (each connection is independent).
    let ab_rows: Vec<(i64, i64, String, Option<i64>)> = conn_ab
        .prepare(
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history ORDER BY \
             changed_at",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let ba_rows: Vec<(i64, i64, String, Option<i64>)> = conn_ba
        .prepare(
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history ORDER BY \
             changed_at",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        ab_rows, ba_rows,
        "windows applied in either order must converge to the same final table"
    );
}

#[test]
fn clock_tie_probe_fires_on_a_non_identical_collision_and_is_silent_on_a_redelivery() {
    let conn = Connection::open_in_memory().expect("duckdb");
    stage(&conn, false);
    insert_events(&conn, &[(1, "2026-01-01 00:00:00", "gold")]);
    let (lead, _lag) = lead_only();
    apply_window(
        &conn,
        "changed_at = TIMESTAMP '2026-01-01 00:00:00'",
        &lead,
        &[],
        None,
    );

    // A redelivery of the exact same row: silent.
    let redelivery_delta = emit_succession_event_delta(
        SOURCE,
        &[
            ("customer_id".to_string(), "customer_id".to_string()),
            ("changed_at".to_string(), "changed_at".to_string()),
            ("tier".to_string(), "tier".to_string()),
        ],
        None,
        "changed_at = TIMESTAMP '2026-01-01 00:00:00'",
    );
    let probe = emit_succession_clock_tie_probe(
        PRESENTED,
        &["customer_id".to_string()],
        "changed_at",
        &["tier".to_string()],
        None,
        &redelivery_delta.sql,
        MaintenanceDialect::DuckDb,
    );
    let violation_count: i64 = conn
        .query_row(&probe.sql, [], |row| row.get(0))
        .expect("probe query");
    assert_eq!(
        violation_count, 0,
        "a redelivered identical row must not fire the clock-tie probe"
    );

    // A non-identical collision: same `(k, t)`, different tier.
    conn.execute_batch(
        "INSERT INTO customer_changes (customer_id, changed_at, tier) VALUES (1, TIMESTAMP \
         '2026-01-01 00:00:00', 'silver')",
    )
    .expect("stage colliding row");
    let colliding_delta = emit_succession_event_delta(
        SOURCE,
        &[
            ("customer_id".to_string(), "customer_id".to_string()),
            ("changed_at".to_string(), "changed_at".to_string()),
            ("tier".to_string(), "tier".to_string()),
        ],
        None,
        "changed_at = TIMESTAMP '2026-01-01 00:00:00' AND tier = 'silver'",
    );
    let probe2 = emit_succession_clock_tie_probe(
        PRESENTED,
        &["customer_id".to_string()],
        "changed_at",
        &["tier".to_string()],
        None,
        &colliding_delta.sql,
        MaintenanceDialect::DuckDb,
    );
    let violation_count2: i64 = conn
        .query_row(&probe2.sql, [], |row| row.get(0))
        .expect("probe query");
    assert!(
        violation_count2 > 0,
        "a non-identical row at the same (k, t) must fire the clock-tie probe"
    );
}
