//! Executability oracle for the four probe emitters
//! `docs/outcomes/20260809-probe-backed-facts/phases/02-plan.md` adds
//! (`emit_functional_dependency_probe`, `emit_bounded_domain_probe`,
//! `emit_monotonicity_probe`, `emit_append_only_posture_probe`): each is
//! proved to actually execute against a real DuckDB and to discriminate
//! conforming from violating data — `emit_statements.rs` only proves the
//! emitted SQL *shape*, never that it runs.

use duckdb::Connection;

use smelt_logical::maintenance::emit::{
    emit_append_only_posture_probe, emit_bounded_domain_probe,
    emit_count_preservation_probe_from_body, emit_functional_dependency_probe,
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
///
/// `check_fingerprint` is applied with the same frontier gate the
/// production `SourcePostureStore::closed_baseline` applies (`docs/
/// outcomes/20260809-probe-backed-facts/outcome.md` phase 6): every
/// partition but the recorded maximum `partition_value` (string
/// comparison) is fingerprint-checked.
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
    let rows: Vec<(String, i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query baseline rows")
        .map(|r| r.expect("baseline row"))
        .collect();
    let max_partition_value = rows
        .iter()
        .map(|(v, _, _)| v.clone())
        .max()
        .unwrap_or_default();
    rows.into_iter()
        .map(|(partition_value, recorded_count, recorded_fingerprint)| {
            let check_fingerprint = partition_value != max_partition_value;
            AppendOnlyBaselinePartition {
                partition_value,
                recorded_count,
                recorded_fingerprint,
                check_fingerprint,
            }
        })
        .collect()
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

/// The frontier gate (`AppendOnlyBaselinePartition::check_fingerprint`): a
/// legitimate append into the still-open (max) partition changes that
/// partition's whole-partition fingerprint, but must not fire — only its
/// count leg is checked, and the count only grew.
#[test]
fn append_into_open_partition_does_not_violate() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE raw_events (event_date DATE, payload TEXT);
         INSERT INTO raw_events VALUES
           (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
    )
    .expect("stage");

    let baseline = current_partition_state(&conn, "raw_events", "event_date", "payload");
    let max_row = baseline
        .iter()
        .find(|b| b.partition_value == "2026-01-02")
        .expect("2026-01-02 is the max partition");
    assert!(
        !max_row.check_fingerprint,
        "the max partition must not be fingerprint-checked"
    );

    // A legitimate append into the still-open max partition.
    conn.execute_batch("INSERT INTO raw_events VALUES (DATE '2026-01-02', 'c');")
        .expect("append");

    let stmt = emit_append_only_posture_probe(
        "raw_events",
        "event_date",
        &["payload".to_string()],
        &baseline,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(
        violation_count, 0,
        "an append into the open partition must hold"
    );
    assert_eq!(sample_keys, None);
}

/// An in-place update of a CLOSED partition (not the recorded maximum)
/// leaves the row count unchanged but changes the fingerprint — the
/// frontier gate admits this partition's fingerprint leg, so it must fire.
#[test]
fn in_place_update_of_closed_partition_violates() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE raw_events (event_date DATE, payload TEXT);
         INSERT INTO raw_events VALUES
           (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
    )
    .expect("stage");

    let baseline = current_partition_state(&conn, "raw_events", "event_date", "payload");
    let closed_row = baseline
        .iter()
        .find(|b| b.partition_value == "2026-01-01")
        .expect("2026-01-01 is a closed partition");
    assert!(
        closed_row.check_fingerprint,
        "a closed partition must be fingerprint-checked"
    );

    // Mutate the closed partition's content in place — same row count.
    conn.execute_batch(
        "UPDATE raw_events SET payload = 'mutated' WHERE event_date = DATE '2026-01-01';",
    )
    .expect("mutate closed partition");

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

/// The count leg is never gated by `check_fingerprint`: a row deleted from
/// the open (max, fingerprint-unchecked) partition still violates via the
/// count leg.
#[test]
fn count_decrease_violates_even_when_fingerprint_unchecked() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE raw_events (event_date DATE, payload TEXT);
         INSERT INTO raw_events VALUES
           (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b'), (DATE '2026-01-02', 'c');",
    )
    .expect("stage");

    let baseline = current_partition_state(&conn, "raw_events", "event_date", "payload");
    let max_row = baseline
        .iter()
        .find(|b| b.partition_value == "2026-01-02")
        .expect("2026-01-02 is the max partition");
    assert!(
        !max_row.check_fingerprint,
        "the max partition must not be fingerprint-checked"
    );

    // Delete a row from the open, fingerprint-unchecked partition.
    conn.execute_batch(
        "DELETE FROM raw_events WHERE event_date = DATE '2026-01-02' AND payload = 'c';",
    )
    .expect("delete from open partition");

    let stmt = emit_append_only_posture_probe(
        "raw_events",
        "event_date",
        &["payload".to_string()],
        &baseline,
        MaintenanceDialect::DuckDb,
    );
    let (violation_count, sample_keys) = probe_result(&conn, &stmt.sql);
    assert_eq!(
        violation_count, 1,
        "a count decrease must violate even though the fingerprint leg is unchecked"
    );
    let sample = sample_keys.expect("expected sample keys for a violation");
    assert!(
        sample.contains("2026-01-02"),
        "expected offending partition `2026-01-02` in: {sample}"
    );
}

// ---------------------------------------------------------------------------
// Count-preservation probe (built from a model's own compiled body,
// `emit_count_preservation_probe_from_body` — `docs/outcomes/
// 20260809-probe-backed-facts/phases/03-plan.md` test 4)
// ---------------------------------------------------------------------------

fn count_preservation_result(conn: &Connection, sql: &str) -> (i64, i64) {
    conn.query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("count-preservation probe")
}

#[test]
fn count_preservation_probe_reports_equal_counts_on_conforming_data() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE main.fact (id INT, dim_id INT);
         CREATE TABLE main.dim (id INT);
         INSERT INTO main.fact VALUES (1, 1), (2, 2);
         INSERT INTO main.dim VALUES (1), (2);",
    )
    .expect("stage");

    let body = "SELECT f.id FROM main.fact f JOIN main.dim d ON f.dim_id = d.id";
    let stmt = emit_count_preservation_probe_from_body(body, "main.dim")
        .expect("body has a top-level join against main.dim");
    let (driving_count, enriched_count) = count_preservation_result(&conn, &stmt.sql);
    assert_eq!(driving_count, enriched_count);
}

#[test]
fn count_preservation_probe_reports_a_shortfall_when_a_fact_key_is_dangling() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE main.fact (id INT, dim_id INT);
         CREATE TABLE main.dim (id INT);
         INSERT INTO main.fact VALUES (1, 1), (2, 99);
         INSERT INTO main.dim VALUES (1);",
    )
    .expect("stage (fact row 2's dim_id 99 has no matching dim row)");

    let body = "SELECT f.id FROM main.fact f JOIN main.dim d ON f.dim_id = d.id";
    let stmt = emit_count_preservation_probe_from_body(body, "main.dim")
        .expect("body has a top-level join against main.dim");
    let (driving_count, enriched_count) = count_preservation_result(&conn, &stmt.sql);
    assert_eq!(driving_count, 2, "both fact rows are the driving side");
    assert_eq!(
        enriched_count, 1,
        "the inner join drops the row whose dim_id has no matching dim row"
    );
    assert!(enriched_count < driving_count);
}
