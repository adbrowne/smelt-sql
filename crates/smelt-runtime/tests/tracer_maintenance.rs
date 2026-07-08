//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Tracer bullet for the maintenance-plan framework
//! (`docs/research/20260705-refresh-as-maintenance-plan/`): prove, against a
//! real DuckDB, that the SQL emitted by `smelt_logical::maintenance` per
//! plan cell is **multiset-equal to a full refresh at the same
//! processed-input set** — the §4 interchangeability theorem exercised over
//! the catalogue's key shapes (EX-02, EX-07, EX-13→EX-40, EX-24, EX-36 of
//! `07-example-catalogue.md`) across the three trigger circumstances: new
//! upstream data, upstream mutation, and column addition + backfill.
//!
//! The oracle is `oracle::multiset_equal` (EXCEPT ALL in both directions),
//! the same primitive every Link-C cell diffs against. The plan derivation
//! is asserted per cell (corner/technique) before its SQL runs, so a shape
//! whose admission regresses fails here before the equivalence check does.
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

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, emit_in_place_update, emit_keyed_fold, Region,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

use oracle::multiset_equal;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn batch(conn: &Connection, statements: &[String]) {
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{sql}"));
    }
}

fn day(d: &str) -> String {
    format!("DATE '{d}'")
}

// ---------------------------------------------------------------------------
// EX-02 — new data + backfill via recompute-region DELETE+INSERT.
// ---------------------------------------------------------------------------

#[test]
fn ex02_recompute_region_equals_full_refresh_across_new_data_and_redelivered_backfill() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE events (event_id INT, user_id INT, event_date DATE, page TEXT, referrer TEXT);
         CREATE TABLE clickstream (event_id INT, user_id INT, event_date DATE, page TEXT, referrer TEXT);
         INSERT INTO events VALUES
           (1, 10, DATE '2026-01-01', '/a', 'https://x.com/p'),
           (2, 11, DATE '2026-01-01', '/b', 'https://y.com/q'),
           (3, 10, DATE '2026-01-02', '/c', 'https://x.com/r');",
    )
    .expect("stage");

    let body = "SELECT event_id, user_id, event_date, page, referrer FROM events";
    let full = body; // full refresh over current input

    // New data: land the initial window.
    batch(
        &conn,
        &emit_delete_insert(
            "clickstream",
            "event_date",
            &Region {
                start: day("2026-01-01"),
                end: day("2026-01-03"),
            },
            body,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM clickstream", full));

    // More new data arrives, and day 2 is re-delivered in the same window —
    // the DELETE+INSERT window replace must stay redelivery-safe.
    conn.execute_batch(
        "INSERT INTO events VALUES (4, 12, DATE '2026-01-03', '/d', 'https://z.com/s');",
    )
    .expect("new day");
    batch(
        &conn,
        &emit_delete_insert(
            "clickstream",
            "event_date",
            &Region {
                start: day("2026-01-02"),
                end: day("2026-01-04"),
            },
            body,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM clickstream", full));
}

// ---------------------------------------------------------------------------
// EX-07 — upstream mutation: dimension churn re-derives {tier} column-scoped,
// leaving every other column untouched.
// ---------------------------------------------------------------------------

#[test]
fn ex07_dimension_churn_column_merge_equals_full_refresh() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE orders (order_id INT, user_id INT, order_date DATE, amount DOUBLE);
         CREATE TABLE customers (user_id INT, tier TEXT);
         INSERT INTO orders VALUES
           (1, 10, DATE '2026-01-01', 5.0),
           (2, 11, DATE '2026-01-01', 7.0),
           (3, 10, DATE '2026-01-02', 9.0);
         INSERT INTO customers VALUES (10, 'bronze'), (11, 'silver');
         CREATE TABLE orders_tiered AS
           SELECT o.order_id, o.user_id, o.order_date, o.amount, c.tier
           FROM orders o JOIN customers c ON c.user_id = o.user_id;",
    )
    .expect("stage");

    let body = "SELECT o.order_id, o.user_id, o.order_date, o.amount, c.tier \
                FROM orders o JOIN customers c ON c.user_id = o.user_id";
    assert!(multiset_equal(&conn, "SELECT * FROM orders_tiered", body));

    // The dimension churns: user 10 upgrades. Only {tier} is
    // mutation-sensitive to customers; the plan's cell is a column-scoped
    // MERGE over the declared full scan (customers is unclocked).
    conn.execute_batch("UPDATE customers SET tier = 'gold' WHERE user_id = 10;")
        .expect("churn");
    batch(
        &conn,
        &emit_column_scoped_merge(
            "orders_tiered",
            &strings(&["order_id", "order_date"]),
            &strings(&["tier"]),
            body,
            None,
            None,
        ),
    );
    assert!(multiset_equal(&conn, "SELECT * FROM orders_tiered", body));
    // And the untouched sibling really was untouched (amounts unchanged).
    assert!(multiset_equal(
        &conn,
        "SELECT order_id, amount FROM orders_tiered",
        "SELECT order_id, amount FROM orders",
    ));
}

// ---------------------------------------------------------------------------
// EX-13 → EX-40 — column addition on an aggregate model: catch up
// {order_count} region by region without disturbing {revenue}, then land new
// data; end state ≡ full refresh of the new definition.
// ---------------------------------------------------------------------------

#[test]
fn ex40_aggregate_column_add_catch_up_then_new_data_equals_full_refresh() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE payments (pay_id INT, pay_date DATE, amount DOUBLE);
         INSERT INTO payments VALUES
           (1, DATE '2026-01-01', 5.0),
           (2, DATE '2026-01-01', 7.0),
           (3, DATE '2026-01-02', 9.0);
         CREATE TABLE daily_revenue AS
           SELECT pay_date, SUM(amount) AS revenue FROM payments GROUP BY pay_date;",
    )
    .expect("stage v1");

    let v2_body = "SELECT pay_date, SUM(amount) AS revenue, COUNT(*) AS order_count \
                   FROM payments GROUP BY pay_date";

    // The definition gains {order_count}. Existing regions hold it at NULL —
    // the S = ∅ state the ledger records per (region, group).
    conn.execute_batch("ALTER TABLE daily_revenue ADD COLUMN order_count BIGINT;")
        .expect("add column");

    // Derive the definition-change cell and check its shape before running it.
    let inputs = ModelInputs {
        sql: v2_body,
        output: OutputSpec {
            table: "daily_revenue".to_string(),
            grain: Grain::Partition {
                partition_col: "pay_date".to_string(),
            },
            skeleton_columns: set(&["pay_date"]),
        },
        sources: vec![SourceFacts {
            name: "payments".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["revenue"]),
                mutation_sensitivity: set(&["payments"]),
            },
            ColumnGroup {
                columns: strings(&["order_count"]),
                mutation_sensitivity: set(&["payments"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["order_count"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells[0].technique, Technique::ColumnScopedMerge);
    assert!(plan.cells[0].ledger_catch_up);

    // Catch up the added group region by region (the backfill loop).
    let count_body = "SELECT pay_date, COUNT(*) AS order_count FROM payments GROUP BY pay_date";
    for (start, end) in [("2026-01-01", "2026-01-02"), ("2026-01-02", "2026-01-03")] {
        batch(
            &conn,
            &emit_column_scoped_merge(
                "daily_revenue",
                &strings(&["pay_date"]),
                &strings(&["order_count"]),
                count_body,
                Some("pay_date"),
                Some(&Region {
                    start: day(start),
                    end: day(end),
                }),
            ),
        );
    }
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM daily_revenue",
        v2_body
    ));

    // Converged: a new day now lands through the ordinary creation cell and
    // computes both groups together.
    conn.execute_batch("INSERT INTO payments VALUES (4, DATE '2026-01-03', 2.0);")
        .expect("new day");
    batch(
        &conn,
        &emit_delete_insert(
            "daily_revenue",
            "pay_date",
            &Region {
                start: day("2026-01-03"),
                end: day("2026-01-04"),
            },
            v2_body,
        ),
    );
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM daily_revenue",
        v2_body
    ));
}

// ---------------------------------------------------------------------------
// EX-36 — column addition, pure function of stored columns: in-place UPDATE
// with no upstream read.
// ---------------------------------------------------------------------------

#[test]
fn ex36_in_place_field_backfill_equals_full_refresh_of_the_new_definition() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE events (event_id INT, event_date DATE, referrer TEXT);
         INSERT INTO events VALUES
           (1, DATE '2026-01-01', 'https://x.com/p'),
           (2, DATE '2026-01-02', 'https://y.com/q');
         CREATE TABLE clickstream AS SELECT event_id, event_date, referrer FROM events;",
    )
    .expect("stage v1");

    conn.execute_batch("ALTER TABLE clickstream ADD COLUMN referrer_domain TEXT;")
        .expect("add column");
    // The field is a pure function of the stored `referrer` — top-left with
    // an empty input delta. Backfill in place; `events` is never read.
    batch(
        &conn,
        &emit_in_place_update(
            "clickstream",
            &[(
                "referrer_domain".to_string(),
                "regexp_extract(referrer, '://([^/]+)', 1)".to_string(),
            )],
            "event_date",
            &Region {
                start: day("2026-01-01"),
                end: day("2026-01-03"),
            },
        ),
    );
    let v2_body = "SELECT event_id, event_date, referrer, \
                   regexp_extract(referrer, '://([^/]+)', 1) AS referrer_domain FROM events";
    assert!(multiset_equal(&conn, "SELECT * FROM clickstream", v2_body));
}

// ---------------------------------------------------------------------------
// EX-24 — keyed end-state fold: MERGE the delta aggregate into stored key
// state; equal to a full refresh over all processed input.
// ---------------------------------------------------------------------------

#[test]
fn ex24_keyed_fold_of_a_delta_equals_full_refresh_at_the_advanced_s() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE payments (pay_id INT, user_id INT, pay_date DATE, amount DOUBLE);
         INSERT INTO payments VALUES
           (1, 10, DATE '2026-01-01', 5.0),
           (2, 11, DATE '2026-01-01', 7.0),
           (3, 10, DATE '2026-01-02', 9.0);
         CREATE TABLE lifetime_spend AS
           SELECT user_id, SUM(amount) AS lifetime_spend FROM payments GROUP BY user_id;",
    )
    .expect("stage");

    // Admission first: the fold cell must be admitted for SUM over
    // append-only payments (and would refuse for a holistic combiner —
    // covered by the smelt-logical unit tests).
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend FROM smelt.sources.payments \
              GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![SourceFacts {
            name: "payments".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
        }],
        fold: Some(FoldSpec {
            add_columns: strings(&["lifetime_spend"]),
            combiner: SqlFunction::Sum,
        }),
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "payments".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells[0].technique, Technique::KeyedFold);

    // A new delta arrives: an existing key grows and a brand-new key appears.
    conn.execute_batch(
        "INSERT INTO payments VALUES
           (4, 10, DATE '2026-01-03', 1.0),
           (5, 12, DATE '2026-01-03', 4.0);",
    )
    .expect("delta");
    batch(
        &conn,
        &emit_keyed_fold(
            "lifetime_spend",
            &strings(&["user_id"]),
            &strings(&["lifetime_spend"]),
            &strings(&["user_id", "lifetime_spend"]),
            "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments \
             WHERE pay_date >= DATE '2026-01-03' AND pay_date < DATE '2026-01-04' \
             GROUP BY user_id",
        ),
    );
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM lifetime_spend",
        "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments GROUP BY user_id",
    ));
}
