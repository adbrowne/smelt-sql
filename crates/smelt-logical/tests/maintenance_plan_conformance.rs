//! `described_technique_matches_execution`: for a representative partition-
//! grain and key-grain shape, the technique `derive_maintenance_plan` derives
//! is asserted *before* its corresponding `maintenance::emit` SQL runs
//! against a real DuckDB — so a plan whose admission regresses (the wrong
//! corner, the wrong technique) fails here before the multiset-equivalence
//! check even executes, and a technique whose emitted SQL is NOT
//! multiset-equal to a full refresh at the same processed-input set fails
//! too. This is the production derivation's own conformance leg
//! (`docs/plans/20260707-maintenance-plan-impl.md` phase MP5); it proves the
//! *description*, not an aspiration, of what a maintenance run does today.
//!
//! Scope, plainly stated: this file is a **reduced-scope placeholder**, not a
//! full conformance suite. Its two cases (EX-02, EX-24) are simplified
//! near-duplicates of coverage `crates/smelt-runtime/tests/tracer_maintenance.rs`
//! already has; it exists only because there is no real `execute_project`
//! consumer of the derived plan yet to diff against (see the judgment call
//! below). Until such a consumer lands, this file adds no coverage beyond
//! what `tracer_maintenance.rs` already provides for these two shapes.
//!
//! Judgment call (documented per the phase's own escape hatch): the plan
//! asked for this comparison against `execute_project`'s DuckDB leg
//! specifically. `execute_project` lives in `smelt-runtime`, which already
//! depends on `smelt-logical` — but `resolve_strategy` still returns a
//! constant (`maintenance_plan.md` §Known Divergences "The plan is
//! specified-and-unwired"), so `execute_project` does not yet consult
//! `derive_maintenance_plan` for *any* model; there is nothing live to diff
//! the derived technique against inside `execute_project` yet, only inside
//! `smelt-runtime`'s own tracer suite (`tests/tracer_maintenance.rs`), which
//! proves the identical property (derive → assert cell shape → emit → real
//! DuckDB → multiset-equal to a full refresh) by calling
//! `smelt_logical::maintenance::{derive, emit}` directly against a raw
//! connection, with no dependency on `execute_project` either. This file
//! ports two of those shapes (EX-02 partition-grain recompute, EX-24
//! key-grain fold) into `smelt-logical`'s own test suite via the same
//! pattern smelt-db already uses for its DuckDB-backed dev-dependencies
//! (`smelt-db` dev-depends on `smelt-runtime`, its own downstream consumer,
//! for exactly this reason) — `smelt-logical` dev-depends on `duckdb`
//! directly (no cycle: `smelt-runtime` is not on the path). This is the most
//! spec-faithful stand-in for "matches execution" available while
//! `execute_project` itself has no maintenance-plan consumer to diff
//! against; MP6+ wires the Salsa/diagnostics path, at which point a real
//! `execute_project`-based comparison becomes possible.
//!
//! Skips loudly (never silently) when `DUCKDB_LIB_DIR`/the system DuckDB
//! library is unavailable: this whole crate's dev-profile already requires
//! it to *compile* (the `duckdb` dev-dependency links against it, same as
//! `smelt-db`'s and `smelt-runtime`'s dev-profiles) — an unset
//! `DUCKDB_LIB_DIR` fails the build with a linker error naming the missing
//! library, not a quietly-green test run.

use std::collections::BTreeSet;

use duckdb::Connection;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::emit::{emit_delete_insert, emit_keyed_fold, Region};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn day(d: &str) -> String {
    format!("DATE '{d}'")
}

/// The Link-C oracle (`crates/smelt-runtime/tests/oracle/mod.rs`, duplicated
/// here for the same reason that file's own doc comment gives — each
/// integration-test file compiles as an independent binary): two relations
/// are equal multisets iff `EXCEPT ALL` is empty in both directions.
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

// ---------------------------------------------------------------------------
// EX-02 — partition grain, new data: recompute-region DELETE+INSERT.
// ---------------------------------------------------------------------------

#[test]
fn described_technique_matches_execution_partition_recompute() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE events (event_id INT, user_id INT, event_date DATE, page TEXT);
         CREATE TABLE clickstream (event_id INT, user_id INT, event_date DATE, page TEXT);
         INSERT INTO events VALUES
           (1, 10, DATE '2026-01-01', '/a'),
           (2, 11, DATE '2026-01-01', '/b');",
    )
    .expect("stage");

    let inputs = ModelInputs {
        sql: "SELECT event_id, user_id, event_date, page FROM smelt.sources.events",
        output: OutputSpec {
            table: "clickstream".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: set(&["event_id", "event_date"]),
        },
        sources: vec![SourceFacts {
            name: "events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["user_id", "page"]),
            mutation_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "events".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    // The described technique: a partition-grain creation trigger recomputes
    // its region via DELETE+INSERT — assert this *before* running any SQL.
    assert_eq!(plan.cells[0].technique, Technique::DeleteInsert);

    let body = "SELECT event_id, user_id, event_date, page FROM events";
    batch(
        &conn,
        &emit_delete_insert(
            "clickstream",
            "event_date",
            &Region {
                start: day("2026-01-01"),
                end: day("2026-01-02"),
            },
            body,
        ),
    );
    // The technique the plan described actually reproduces a full refresh.
    assert!(multiset_equal(&conn, "SELECT * FROM clickstream", body));
}

// ---------------------------------------------------------------------------
// EX-24 — key grain, new data: fold-a-delta into keyed end-state.
// ---------------------------------------------------------------------------

#[test]
fn described_technique_matches_execution_keyed_fold() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE payments (pay_id INT, user_id INT, pay_date DATE, amount DOUBLE);
         INSERT INTO payments VALUES
           (1, 10, DATE '2026-01-01', 5.0),
           (2, 11, DATE '2026-01-01', 7.0);
         CREATE TABLE lifetime_spend AS
           SELECT user_id, SUM(amount) AS lifetime_spend FROM payments GROUP BY user_id;",
    )
    .expect("stage");

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
    // The described technique: a key-grain creation trigger over an
    // append-only source and a SUM combiner folds the delta into stored
    // state — assert this *before* running any SQL.
    assert_eq!(plan.cells[0].technique, Technique::KeyedFold);

    conn.execute_batch("INSERT INTO payments VALUES (3, 10, DATE '2026-01-02', 2.0);")
        .expect("delta");
    batch(
        &conn,
        &emit_keyed_fold(
            "lifetime_spend",
            &strings(&["user_id"]),
            &strings(&["lifetime_spend"]),
            &strings(&["user_id", "lifetime_spend"]),
            "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments \
             WHERE pay_date >= DATE '2026-01-02' AND pay_date < DATE '2026-01-03' \
             GROUP BY user_id",
        ),
    );
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM lifetime_spend",
        "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments GROUP BY user_id",
    ));
}

fn batch(conn: &Connection, statements: &[String]) {
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{sql}"));
    }
}
