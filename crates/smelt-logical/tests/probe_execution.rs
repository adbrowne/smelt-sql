//! Executability oracle for the four probe emitters
//! `docs/outcomes/20260809-probe-backed-facts/phases/02-plan.md` adds
//! (`emit_functional_dependency_probe`, `emit_bounded_domain_probe`,
//! `emit_monotonicity_probe`, `emit_append_only_posture_probe`): each is
//! proved to actually execute against a real DuckDB and to discriminate
//! conforming from violating data — `emit_statements.rs` only proves the
//! emitted SQL *shape*, never that it runs.

use duckdb::Connection;

use smelt_logical::maintenance::emit::{
    emit_append_only_posture_probe, emit_bounded_domain_probe, emit_functional_dependency_probe,
    emit_monotonicity_probe, AppendOnlyBaselinePartition, MaintenanceDialect,
};

fn probe_result(conn: &Connection, sql: &str) -> (i64, Option<String>) {
    conn.query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("probe query")
}

// ---------------------------------------------------------------------------
// Functional-dependency probe
// ---------------------------------------------------------------------------

#[test]
fn fd_probe_returns_zero_on_conforming_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE subscriptions (customer_id INT, plan_tier TEXT);
         INSERT INTO subscriptions VALUES (1, 'gold'), (1, 'gold'), (2, 'silver');",
    )
    .expect("stage");

    let stmt = emit_functional_dependency_probe(
        "SELECT customer_id, plan_tier FROM subscriptions",
        &["customer_id".to_string()],
        "plan_tier",
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 0);
    assert_eq!(sample_keys, None);
}

#[test]
fn fd_probe_returns_nonzero_with_samples_on_violating_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE subscriptions (customer_id INT, plan_tier TEXT);
         INSERT INTO subscriptions VALUES (1, 'gold'), (1, 'silver'), (2, 'silver');",
    )
    .expect("stage");

    let stmt = emit_functional_dependency_probe(
        "SELECT customer_id, plan_tier FROM subscriptions",
        &["customer_id".to_string()],
        "plan_tier",
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 1);
    let sample = sample_keys.expect("expected sample keys for a violation");
    assert!(
        sample.contains('1'),
        "expected offending key `1` in: {sample}"
    );
}

// ---------------------------------------------------------------------------
// Bounded-domain probe
// ---------------------------------------------------------------------------

#[test]
fn bounded_domain_probe_returns_zero_on_conforming_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE shipments (country_code TEXT);
         INSERT INTO shipments VALUES ('US'), ('US'), ('CA');",
    )
    .expect("stage");

    let stmt = emit_bounded_domain_probe(
        "SELECT country_code FROM shipments",
        "country_code",
        5,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 0);
    assert_eq!(sample_keys, None);
}

#[test]
fn bounded_domain_probe_returns_nonzero_with_samples_on_violating_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE shipments (country_code TEXT);
         INSERT INTO shipments VALUES ('US'), ('CA'), ('MX'), ('FR');",
    )
    .expect("stage");

    let stmt = emit_bounded_domain_probe(
        "SELECT country_code FROM shipments",
        "country_code",
        2,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 4);
    assert!(
        sample_keys.is_some(),
        "expected sample values for a violation"
    );
}

// ---------------------------------------------------------------------------
// Monotonicity probe
// ---------------------------------------------------------------------------

#[test]
fn monotonicity_probe_returns_zero_on_conforming_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE events (user_id INT, event_time TIMESTAMP);
         INSERT INTO events VALUES
           (1, TIMESTAMP '2026-01-01 00:00:00'),
           (1, TIMESTAMP '2026-01-01 01:00:00'),
           (2, TIMESTAMP '2026-01-01 00:30:00');",
    )
    .expect("stage");

    let stmt = emit_monotonicity_probe(
        "SELECT user_id, event_time FROM events",
        &["user_id".to_string()],
        "event_time",
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 0);
    assert_eq!(sample_keys, None);
}

#[test]
fn monotonicity_probe_returns_nonzero_with_samples_on_violating_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE events (user_id INT, event_time TIMESTAMP);
         INSERT INTO events VALUES
           (1, TIMESTAMP '2026-01-01 01:00:00'),
           (1, TIMESTAMP '2026-01-01 00:00:00');",
    )
    .expect("stage");

    let stmt = emit_monotonicity_probe(
        "SELECT user_id, event_time FROM events",
        &["user_id".to_string()],
        "event_time",
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 1);
    let sample = sample_keys.expect("expected sample keys for a violation");
    assert!(
        sample.contains('1'),
        "expected offending partition `1` in: {sample}"
    );
}

// ---------------------------------------------------------------------------
// Append-only posture probe
// ---------------------------------------------------------------------------

/// Queries the CURRENT per-partition count and content fingerprint using
/// the same construction the emitter builds internally (DuckDB dialect: a
/// `STRING_AGG`-of-sorted-row-hashes aggregate) — the test's own baseline
/// setup, deliberately independent of the emitter's private helpers, so the
/// oracle doesn't just compare the emitter against itself.
fn current_partition_state(
    conn: &Connection,
    table: &str,
    partition_column: &str,
    digest_column: &str,
) -> Vec<AppendOnlyBaselinePartition> {
    // `row_hash` mirrors `emit_append_only_posture_probe`'s own
    // `row_fingerprint_expr`: `sha256` of `concat_varchar_expr_typed`'s
    // output, which for a single digest column is *already* a `sha256`
    // digest ([`column_fingerprint_expr`]) — so a single-column row hash is
    // deliberately double-hashed, matching production exactly (the same
    // convention `emit_fingerprint_digest_select` has always used).
    let sql = format!(
        "SELECT CAST({partition_column} AS VARCHAR) AS partition_value, COUNT(*) AS current_count, \
         sha256(STRING_AGG(row_hash, '' ORDER BY row_hash)) AS current_fingerprint \
         FROM (SELECT {partition_column}, \
               sha256(sha256(CASE WHEN {digest_column} IS NULL THEN 'N' \
                      ELSE CONCAT('V', CAST({digest_column} AS VARCHAR)) END)) AS row_hash \
               FROM {table}) AS __rows \
         GROUP BY {partition_column}"
    );
    let mut stmt = conn.prepare(&sql).expect("prepare baseline query");
    let rows = stmt
        .query_map([], |row| {
            Ok(AppendOnlyBaselinePartition {
                partition_value: row.get(0)?,
                recorded_count: row.get(1)?,
                recorded_fingerprint: row.get(2)?,
            })
        })
        .expect("query baseline rows");
    rows.map(|r| r.expect("baseline row")).collect()
}

#[test]
fn append_only_posture_probe_returns_zero_on_conforming_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE raw_events (event_date DATE, payload TEXT);
         INSERT INTO raw_events VALUES
           (DATE '2026-01-01', 'a'), (DATE '2026-01-01', 'b'), (DATE '2026-01-02', 'c');",
    )
    .expect("stage");

    let baseline = current_partition_state(&conn, "raw_events", "event_date", "payload");
    assert!(!baseline.is_empty(), "expected a recorded baseline");

    let stmt = emit_append_only_posture_probe(
        "raw_events",
        "event_date",
        &["payload".to_string()],
        &baseline,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 0);
    assert_eq!(sample_keys, None);
}

#[test]
fn append_only_posture_probe_returns_nonzero_with_samples_on_violating_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE raw_events (event_date DATE, payload TEXT);
         INSERT INTO raw_events VALUES
           (DATE '2026-01-01', 'a'), (DATE '2026-01-01', 'b'), (DATE '2026-01-02', 'c');",
    )
    .expect("stage");

    let baseline = current_partition_state(&conn, "raw_events", "event_date", "payload");

    // Simulate an in-place mutation after the baseline was recorded: a row's
    // payload changes without any row count change — a re-check must catch
    // the changed fingerprint even though the row count still matches.
    conn.execute_batch("UPDATE raw_events SET payload = 'mutated' WHERE payload = 'a';")
        .expect("mutate");

    let stmt = emit_append_only_posture_probe(
        "raw_events",
        "event_date",
        &["payload".to_string()],
        &baseline,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(violation_count, 1);
    let sample = sample_keys.expect("expected sample keys for a violation");
    assert!(
        sample.contains("2026-01-01"),
        "expected offending partition `2026-01-01` in: {sample}"
    );
}
