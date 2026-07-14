//! `described_technique_matches_execution`: for a representative partition-
//! grain and key-grain shape (plus a grain-alignment corner, EX-18), the
//! technique `derive_maintenance_plan` derives is asserted *before* its
//! corresponding `maintenance::emit` SQL runs against a real DuckDB — so a
//! plan whose admission regresses (the wrong corner, the wrong technique)
//! fails here before the multiset-equivalence check even executes, and a
//! technique whose emitted SQL is NOT multiset-equal to a full refresh at
//! the same processed-input set fails too. This is the production
//! derivation's own conformance leg (`docs/plans/
//! 20260707-maintenance-plan-impl.md` phase MP5); it proves the
//! *description*, not an aspiration, of what a maintenance run does today.
//!
//! Scope, plainly stated: this file proves the *plan derivation's* chosen
//! technique, called through the single-owner emitters directly against a
//! raw DuckDB connection, reproduces a full refresh for three shapes —
//! EX-02 (partition-grain recompute), EX-24 (key-grain fold), and EX-18
//! (grain-alignment write-window rounding). It does **not** run these
//! statements through `execute_project`: `smelt-logical` sits below
//! `smelt-runtime` in the crate layering (`docs/specs/architecture.md`
//! §"Layered single-ownership"; `smelt-runtime` depends on `smelt-logical`,
//! never the reverse), so this file cannot call `execute_project` itself.
//!
//! The **production-execution** half of "matches execution" — proving the
//! statements a real `execute_project` run actually sends to a live
//! backend are both byte-identical to the emitters' output *and*
//! result-equivalent to a full refresh — is proved in
//! `crates/smelt-runtime/tests/statement_parity.rs`
//! (`region_recompute_statements_come_from_the_emitter`,
//! `keyed_fold_statements_come_from_the_emitter`,
//! `column_scoped_merge_statements_come_from_the_emitter`), which sits
//! above both crates and can call `execute_project`. That file's structural
//! gate (`no_maintenance_statement_authoring_outside_the_emitter`) is the
//! standing CI enforcement of single ownership referenced by
//! `docs/specs/architecture.md` §"Constraints & Invariants" item 12. This
//! file and that one are companions, not duplicates: this file proves the
//! *derivation* picks the right technique and that technique's emitter
//! output reproduces a full refresh; `statement_parity.rs` proves
//! *production execution* actually runs that same emitter output, byte for
//! byte, and that its result also reproduces a full refresh.
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
use smelt_logical::maintenance::emit::{
    emit_delete_insert, emit_keyed_fold, MaintenanceDialect, Region, StatementGroup,
};
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
    let region = Region {
        start: day("2026-01-01"),
        end: day("2026-01-02"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "clickstream",
            "event_date",
            &region,
            &clamped(body, "event_date", &region),
            MaintenanceDialect::DuckDb,
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
    batch_group(
        &conn,
        &emit_keyed_fold(
            "lifetime_spend",
            &strings(&["user_id"]),
            &[(
                "lifetime_spend".to_string(),
                "target.lifetime_spend + delta.lifetime_spend".to_string(),
            )],
            "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments \
             WHERE pay_date >= DATE '2026-01-02' AND pay_date < DATE '2026-01-03' \
             GROUP BY user_id",
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(multiset_equal(
        &conn,
        "SELECT * FROM lifetime_spend",
        "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments GROUP BY user_id",
    ));
}

// ---------------------------------------------------------------------------
// EX-18 — GROUP BY week over day partitions: the write window must round
// up to the containing week (`07-example-catalogue.md` EX-18: "HOLDS
// (recompute of the containing week today, provided the write window is
// widened to whole weeks)").
//
// `maintenance_coverage_matrix.rs::ex18_group_by_coarser_write_window_rounds_up`
// pins the *precondition* this equivalence leg depends on
// (`check_declared_granularity` proves declaring the coarser `week` grain is
// a safe widen, never a narrowing hazard) but does not itself run any SQL.
// This test supplies the missing equivalence leg: it derives the plan
// (asserting the partition-grain recompute technique the catalogue
// predicts), then emits `DeleteInsert` over a region that is the WEEK the
// new day lands in — not the day itself — and proves the result is
// multiset-equal to a full refresh. A second run using a day-scoped (not
// week-rounded) region demonstrates why the widen is load-bearing: the
// narrower region touches zero rows (a day boundary that isn't itself a
// week boundary matches no stored `order_week` row), silently leaving the
// week stale — the exact hazard the write-window-rounds-up guarantee rules
// out.
// ---------------------------------------------------------------------------

#[test]
fn described_technique_matches_execution_ex18_group_by_coarser_write_window() {
    let conn = Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(
        "CREATE TABLE orders (order_id INT, order_ts DATE, amount DOUBLE);
         -- Week of 2026-01-05 (Monday) .. 2026-01-11: two days landed so far.
         INSERT INTO orders VALUES
           (1, DATE '2026-01-05', 100.0),
           (2, DATE '2026-01-06', 50.0);
         -- A different, already-closed week — must be left untouched.
         INSERT INTO orders VALUES
           (3, DATE '2026-01-12', 200.0);
         CREATE TABLE weekly_finance AS
           SELECT date_trunc('week', order_ts) AS order_week, SUM(amount) AS total \
           FROM orders GROUP BY 1;",
    )
    .expect("stage");

    let sql = "SELECT date_trunc('week', order_ts) AS order_week, SUM(amount) AS total \
               FROM smelt.sources.orders GROUP BY 1";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "weekly_finance".to_string(),
            grain: Grain::Partition {
                partition_col: "order_week".to_string(),
            },
            skeleton_columns: set(&["order_week"]),
        },
        sources: vec![SourceFacts {
            name: "orders".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("order_ts".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["total"]),
            mutation_sensitivity: set(&["orders"]),
        }],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "orders".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    // The described technique: a partition-grain creation trigger recomputes
    // its region via DELETE+INSERT — assert this *before* running any SQL.
    assert_eq!(plan.cells[0].technique, Technique::DeleteInsert);

    // A new day lands mid-week (2026-01-07, still within the 01-05..01-11
    // week already partially stored).
    conn.execute_batch("INSERT INTO orders VALUES (4, DATE '2026-01-07', 30.0);")
        .expect("mid-week delta");

    let body = "SELECT date_trunc('week', order_ts) AS order_week, SUM(amount) AS total \
                FROM orders GROUP BY 1";

    // First, demonstrate the hazard: a day-scoped (not week-rounded) region
    // touches zero rows, because no stored `order_week` value falls inside
    // a single day's half-open interval — the write silently no-ops and the
    // week goes stale.
    let day_scoped_region = Region {
        start: day("2026-01-07"),
        end: day("2026-01-08"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "weekly_finance",
            "order_week",
            &day_scoped_region,
            &clamped(body, "order_week", &day_scoped_region),
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(
        !multiset_equal(&conn, "SELECT * FROM weekly_finance", body),
        "a day-scoped region must NOT reproduce a full refresh — it demonstrates why the \
         write window must round up to the week boundary"
    );

    // Now the correct write window: rounded up to the whole week
    // containing the new day. This is the guarantee
    // `check_declared_granularity` (MP14) makes true by construction.
    let week_scoped_region = Region {
        start: day("2026-01-05"),
        end: day("2026-01-12"),
    };
    batch_group(
        &conn,
        &emit_delete_insert(
            "weekly_finance",
            "order_week",
            &week_scoped_region,
            &clamped(body, "order_week", &week_scoped_region),
            MaintenanceDialect::DuckDb,
        ),
    );
    assert!(
        multiset_equal(&conn, "SELECT * FROM weekly_finance", body),
        "recompute of the week-rounded region must reproduce a full refresh"
    );
}

fn batch_group(conn: &Connection, group: &StatementGroup) {
    for stmt in &group.statements {
        conn.execute_batch(&stmt.sql)
            .unwrap_or_else(|e| panic!("statement failed: {e}\n{}", stmt.sql));
    }
}

/// Test-only stand-in for the runtime's output-clamp injection
/// (`smelt-runtime/src/transformer.rs`): the new single-owner emitter
/// contract requires the caller to fold the region predicate into the body
/// it hands `emit_delete_insert` — the emitter itself no longer adds one
/// (`docs/specs/maintenance_plan.md` §"Statement emission (single owner)").
fn clamped(body: &str, col: &str, region: &Region) -> String {
    format!(
        "SELECT * FROM ({body}) WHERE {col} >= {start} AND {col} < {end}",
        start = region.start,
        end = region.end,
    )
}

// =============================================================================
// `coverage_matrix_is_inhabited` — the standing inventory gate
// (`docs/plans/20260707-maintenance-plan-impl.md` phase MP17).
//
// Encodes the research coverage matrix
// (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`
// §"Coverage matrix": 21 construct rows × 7 source-property columns, plus a
// 22nd row this phase adds — `INTERSECT`/`EXCEPT` — to pin the set-op
// classification gap the matrix itself doesn't have a row for) as data, and
// asserts every INHABITED cell (the catalogue names an example for it) is
// accounted for by exactly one of two explicit, disjoint lists:
//
// - `CLAIMED` — a grounded, executable test exists for this cell (named,
//   with the catalogue id and the test that proves it).
// - `KNOWN_GAPS` — this phase did NOT reach this cell; named so a future
//   phase inherits an explicit backlog, never a silent hole.
//
// Additive-only enforcement: `MATRIX` is the literal transcription of the
// catalogue table. Adding a new inhabited cell to `MATRIX` without adding a
// matching entry to `CLAIMED` or `KNOWN_GAPS` fails `every_inhabited_cell_is_accounted_for`
// — there is no way to add matrix coverage silently.
// =============================================================================

/// One coverage-matrix cell: `None` = "—" (not inhabited by the catalogue).
type Cell = Option<&'static str>;

/// `(construct row name, [append-only, append-only+lateness, mutable
/// snapshot, change feed (retractions), at-least-once redelivery, unclocked
/// lookup/dim, composite key])` — column order matches the catalogue table
/// header exactly.
const MATRIX: &[(&str, [Cell; 7])] = &[
    (
        "pass-through projection",
        [
            Some("EX-02"),
            Some("EX-03"),
            Some("EX-04"),
            Some("EX-14"),
            Some("EX-05"),
            None,
            None,
        ],
    ),
    (
        "additive agg (SUM/COUNT)",
        [
            Some("EX-13"),
            Some("EX-18"),
            Some("EX-04"),
            Some("EX-14"),
            Some("EX-20"),
            None,
            None,
        ],
    ),
    (
        "idempotent agg (MIN/MAX/BOOL_OR)",
        [
            Some("EX-15"),
            Some("EX-15"),
            Some("EX-16"),
            Some("EX-16"),
            Some("EX-15"),
            None,
            None,
        ],
    ),
    (
        "holistic agg (MEDIAN/COUNT DISTINCT)",
        [
            Some("EX-17"),
            Some("EX-17"),
            Some("EX-17"),
            None,
            Some("EX-17"),
            None,
            None,
        ],
    ),
    (
        "inner-join enrichment",
        [
            Some("EX-08"),
            Some("EX-08"),
            Some("EX-07"),
            Some("EX-26"),
            None,
            Some("EX-07/EX-08"),
            Some("EX-10"),
        ],
    ),
    (
        "LEFT JOIN (null-preservation)",
        [Some("EX-09"), Some("EX-09"), None, None, None, None, None],
    ),
    (
        "join fan-out (1:N / N:1 proof)",
        [
            Some("EX-10"),
            None,
            Some("EX-10"),
            None,
            None,
            Some("EX-10"),
            Some("EX-10"),
        ],
    ),
    (
        "correlated EXISTS / scalar subquery",
        [
            Some("EX-01/EX-11"),
            Some("EX-01"),
            Some("EX-16"),
            None,
            None,
            None,
            None,
        ],
    ),
    (
        "correlated first-value pick (MIN_BY / first)",
        [Some("EX-35"), Some("EX-35"), None, None, None, None, None],
    ),
    (
        "window: running total (trajectory)",
        [Some("EX-22"), Some("EX-23"), None, None, None, None, None],
    ),
    (
        "window: LAG/LEAD",
        [Some("EX-25"), Some("EX-25"), None, None, None, None, None],
    ),
    (
        "window: ROW_NUMBER dedup",
        [
            Some("EX-27"),
            Some("EX-27"),
            None,
            None,
            Some("EX-27"),
            None,
            None,
        ],
    ),
    (
        "UNION ALL",
        [
            Some("EX-05"),
            Some("EX-05"),
            Some("EX-06"),
            None,
            None,
            None,
            None,
        ],
    ),
    (
        "self-referential model",
        [Some("EX-21"), Some("EX-21"), None, None, None, None, None],
    ),
    (
        "GROUP BY coarser than partition",
        [Some("EX-18"), Some("EX-18"), None, None, None, None, None],
    ),
    (
        "multi-input column group (merge)",
        [
            None,
            None,
            Some("EX-12"),
            Some("EX-12"),
            None,
            Some("EX-12"),
            None,
        ],
    ),
    (
        "dedup-to-latest (keyed collapse)",
        [
            Some("EX-27"),
            Some("EX-27"),
            None,
            None,
            Some("EX-27"),
            None,
            None,
        ],
    ),
    (
        "keyed end-state fold",
        [
            Some("EX-19/EX-24"),
            Some("EX-24"),
            None,
            Some("EX-26"),
            None,
            None,
            None,
        ],
    ),
    (
        "SCD2 / versioned intervals",
        [None, None, Some("EX-29"), Some("EX-28"), None, None, None],
    ),
    (
        "engine-maintained (MV)",
        [Some("EX-32"), None, Some("EX-32"), None, None, None, None],
    ),
    (
        "cross-model DAG propagation",
        [
            Some("EX-31/EX-33/EX-34"),
            Some("EX-34"),
            None,
            None,
            None,
            None,
            None,
        ],
    ),
    // Not a row of the research catalogue's own matrix table — added by this
    // phase (`maintenance_plan.md` §Known Divergences "INTERSECT/EXCEPT are
    // unclassified set operations") to give the set-op classification gap a
    // named cell rather than leaving it matrix-invisible. The collapse this
    // pins is source-property-agnostic (grouping fails closed the same way
    // regardless of which source is append-only/mutable/etc — see
    // `coverage_matrix_gaps.rs::ex41_ex42_intersect_with_payload_column_collapses_whole_model`),
    // so only the `append-only` column is marked inhabited: one cell is
    // enough to name the gap without implying the other six columns carry
    // independently distinct verdicts.
    (
        "INTERSECT / EXCEPT (set operations)",
        [Some("EX-41/EX-42"), None, None, None, None, None, None],
    ),
];

/// Cells this phase (MP17) reaches with a grounded, executable test —
/// `(row name, column index, note)`. Column indices match `MATRIX`'s header:
/// 0=append-only, 1=append-only+lateness, 2=mutable snapshot, 3=change feed,
/// 4=at-least-once redelivery, 5=unclocked lookup/dim, 6=composite key.
const CLAIMED: &[(&str, usize, &str)] = &[
    (
        "pass-through projection",
        0,
        "maintenance_plan_conformance.rs::described_technique_matches_execution_partition_recompute (EX-02, HOLDS); production-execution byte+result parity via crates/smelt-runtime/tests/statement_parity.rs::region_recompute_statements_come_from_the_emitter",
    ),
    (
        "additive agg (SUM/COUNT)",
        3,
        "maintenance_coverage_matrix.rs::ex14_change_feed_sum_recompute_only (EX-14, refuses fold / recompute-only)",
    ),
    (
        "additive agg (SUM/COUNT)",
        1,
        "described_technique_matches_execution_ex18_group_by_coarser_write_window (EX-18, HOLDS — recompute over the week-rounded region, proved equivalent to full refresh); the region DELETE+INSERT family's production-execution byte+result parity is grounded generically (not EX-18's specific week-rounding corner) via crates/smelt-runtime/tests/statement_parity.rs::region_recompute_statements_come_from_the_emitter",
    ),
    (
        "inner-join enrichment",
        5,
        "coverage_matrix_gaps.rs::ex08_unclocked_change_feed_dimension_scan_unbounded (EX-08, refuses — ScanUnbounded)",
    ),
    (
        "correlated first-value pick (MIN_BY / first)",
        0,
        "maintenance_coverage_matrix.rs::ex35_correlated_first_value_recompute_only (EX-35, refuses fold / recompute-only)",
    ),
    (
        "window: ROW_NUMBER dedup",
        0,
        "maintenance_coverage_matrix.rs::ex27_row_number_dedup_refuses_today (EX-27, refuses — no fold specification)",
    ),
    (
        "GROUP BY coarser than partition",
        0,
        "described_technique_matches_execution_ex18_group_by_coarser_write_window (EX-18, HOLDS — recompute over the week-rounded region, proved equivalent to full refresh); the region DELETE+INSERT family's production-execution byte+result parity is grounded generically (not EX-18's specific week-rounding corner) via crates/smelt-runtime/tests/statement_parity.rs::region_recompute_statements_come_from_the_emitter",
    ),
    (
        "GROUP BY coarser than partition",
        1,
        "described_technique_matches_execution_ex18_group_by_coarser_write_window (EX-18, HOLDS — recompute over the week-rounded region, proved equivalent to full refresh); the region DELETE+INSERT family's production-execution byte+result parity is grounded generically (not EX-18's specific week-rounding corner) via crates/smelt-runtime/tests/statement_parity.rs::region_recompute_statements_come_from_the_emitter",
    ),
    (
        "multi-input column group (merge)",
        2,
        "maintenance_coverage_matrix.rs::ex12_multi_input_merge_degenerates_to_recompute (EX-12, pins the shared-technique divergence)",
    ),
    (
        "multi-input column group (merge)",
        5,
        "maintenance_coverage_matrix.rs::ex12_multi_input_merge_degenerates_to_recompute (EX-12, fx_rates is the unclocked leg)",
    ),
    (
        "dedup-to-latest (keyed collapse)",
        0,
        "maintenance_coverage_matrix.rs::ex27_row_number_dedup_refuses_today (EX-27, refuses — no fold specification)",
    ),
    (
        "keyed end-state fold",
        0,
        "maintenance_plan_conformance.rs::described_technique_matches_execution_keyed_fold (EX-24, HOLDS); production-execution byte+result parity via crates/smelt-runtime/tests/statement_parity.rs::keyed_fold_statements_come_from_the_emitter",
    ),
    (
        "keyed end-state fold",
        3,
        "maintenance_coverage_matrix.rs::ex26_change_feed_latest_writer_recompute_only (EX-26, refuses fold / recompute-only)",
    ),
    (
        "INTERSECT / EXCEPT (set operations)",
        0,
        "coverage_matrix_gaps.rs::ex41_ex42_intersect_no_payload_column_still_delete_insert + ..._collapses_whole_model (pins the classification-collapse today)",
    ),
];

/// Cells this phase deliberately did NOT reach — named, not omitted. Each
/// entry states in ONE line why it's out of this pass's scope, so a future
/// phase picks up an explicit backlog rather than rediscovering the gap.
/// `(row name, column index, reason)`.
const KNOWN_GAPS: &[(&str, usize, &str)] = &[
    ("pass-through projection", 1, "EX-03: read-modify-write-region vs recompute-region bake-off (§4 interchangeability) — needs a cost-model harness this phase didn't build"),
    ("pass-through projection", 2, "EX-04: mutable-snapshot backfill-recovers story — plausibly covered by the pre-existing `sc_2_clocked_mutable_window_forward` property-discovery probe, not re-verified against this exact catalogue id this pass"),
    ("pass-through projection", 3, "EX-14 dagger (discussion variant, not headline) — see `additive agg` row's claim for the headline cell"),
    ("pass-through projection", 4, "EX-05 dagger (discussion variant) — headline covered under `UNION ALL` row instead"),
    ("additive agg (SUM/COUNT)", 0, "EX-13: plausibly covered by `maintenance_conformance::pinned::hazard::additive_agg_append_only_control` (graduated from the retired `g_01_additive_agg_append_only` property-discovery probe), not re-verified against this exact catalogue id this pass"),
    ("additive agg (SUM/COUNT)", 2, "EX-04 dagger (discussion variant)"),
    ("additive agg (SUM/COUNT)", 4, "EX-20: plausibly covered by `maintenance_conformance::pinned::hazard::additive_agg_redelivery` (graduated from the retired `g_02_additive_agg_redelivery` property-discovery probe), not re-verified against this exact catalogue id this pass"),
    ("idempotent agg (MIN/MAX/BOOL_OR)", 0, "EX-15: plausibly covered by `maintenance_conformance::pinned::hazard::idempotent_agg_append_only_control` (graduated from the retired `g_03_idempotent_agg_append_only` property-discovery probe), not re-verified against this catalogue id"),
    ("idempotent agg (MIN/MAX/BOOL_OR)", 1, "EX-15 dagger (discussion variant)"),
    ("idempotent agg (MIN/MAX/BOOL_OR)", 2, "EX-16: plausibly covered by `g_04_idempotent_min_mutable_snapshot`, not re-verified against this catalogue id"),
    ("idempotent agg (MIN/MAX/BOOL_OR)", 3, "EX-16 dagger (discussion variant)"),
    ("idempotent agg (MIN/MAX/BOOL_OR)", 4, "EX-15 dagger (discussion variant)"),
    ("holistic agg (MEDIAN/COUNT DISTINCT)", 0, "EX-17: plausibly covered by `maintenance_conformance::pinned::hazard::holistic_agg_append_only_control` (graduated from the retired `g_07_holistic_agg_append_only` property-discovery probe), not re-verified against this catalogue id"),
    ("holistic agg (MEDIAN/COUNT DISTINCT)", 1, "EX-17 dagger (discussion variant)"),
    ("holistic agg (MEDIAN/COUNT DISTINCT)", 2, "EX-17 dagger (discussion variant)"),
    ("holistic agg (MEDIAN/COUNT DISTINCT)", 4, "EX-17: same probe as column 0, redelivery variant not isolated"),
    ("inner-join enrichment", 0, "EX-08 headline: needs a genuine append-only + mutable-dimension two-source harness beyond the unclocked-scan-refusal shape this pass built"),
    ("inner-join enrichment", 1, "EX-08 dagger (discussion variant)"),
    ("inner-join enrichment", 2, "EX-07: plausibly covered by `maintenance_conformance::pinned::hazard::join_enrichment_mutable_dimension` (graduated from the retired `g_05_join_enrichment_mutable_dimension` property-discovery probe), not re-verified against this catalogue id"),
    ("inner-join enrichment", 3, "EX-26 dagger (discussion variant) — headline covered under `keyed end-state fold` row instead"),
    ("inner-join enrichment", 6, "EX-10: composite-key inner-join cell — plausibly covered by `maintenance_conformance::pinned::hazard::composite_key_join_fan_out` (graduated from the retired `g_10_composite_key_join_fan_out` property-discovery probe), not re-verified against this catalogue id"),
    ("LEFT JOIN (null-preservation)", 0, "EX-09: plausibly covered by `g_06_left_join_null_preservation`, not re-verified against this catalogue id"),
    ("LEFT JOIN (null-preservation)", 1, "EX-09: same probe, lateness variant not isolated"),
    ("join fan-out (1:N / N:1 proof)", 0, "EX-10: plausibly covered by `maintenance_conformance::pinned::hazard::composite_key_join_fan_out` (graduated from the retired `g_10_composite_key_join_fan_out` property-discovery probe), not re-verified against this catalogue id"),
    ("join fan-out (1:N / N:1 proof)", 2, "EX-10 dagger (discussion variant)"),
    ("join fan-out (1:N / N:1 proof)", 5, "EX-10: unclocked-dim fan-out variant not isolated from the composite-key probe"),
    ("join fan-out (1:N / N:1 proof)", 6, "EX-10: plausibly covered by `maintenance_conformance::pinned::hazard::composite_key_join_fan_out` (graduated from the retired `g_10_composite_key_join_fan_out` property-discovery probe), not re-verified against this catalogue id"),
    ("correlated EXISTS / scalar subquery", 0, "EX-01/EX-11: plausibly covered by `sc_1_correlated_exists`'s recompute arm, not re-verified against this catalogue id; EX-11's additive-combiner fold cell is UNSUPPORTED-TODAY and unbuilt"),
    ("correlated EXISTS / scalar subquery", 1, "EX-01: plausibly covered by `sc_1_correlated_exists`, not re-verified against this catalogue id"),
    ("correlated EXISTS / scalar subquery", 2, "EX-16 dagger (discussion variant)"),
    ("correlated first-value pick (MIN_BY / first)", 1, "EX-35 dagger (discussion variant)"),
    ("window: running total (trajectory)", 0, "EX-22: plausibly covered by `g_08_running_total_self_ref`, not re-verified against this catalogue id"),
    ("window: running total (trajectory)", 1, "EX-23: plausibly covered by `sc_4_stacked_frames`'s reach-composition probe, not re-verified against this catalogue id"),
    ("window: LAG/LEAD", 0, "EX-25: footprint-reflection across a partition boundary — needs `source_bounds` investigation into whether LAG/LEAD offsets derive an `after` margin at all; not attempted this pass"),
    ("window: LAG/LEAD", 1, "EX-25: same gap, lateness variant"),
    ("window: ROW_NUMBER dedup", 1, "EX-27: lateness variant of the refusal this pass already pins at column 0"),
    ("window: ROW_NUMBER dedup", 4, "EX-27: at-least-once-redelivery variant of the refusal this pass already pins at column 0"),
    ("UNION ALL", 0, "EX-05: plausibly covered by `g_09_union_all_append_only`, not re-verified against this catalogue id"),
    ("UNION ALL", 1, "EX-05 dagger (discussion variant)"),
    ("UNION ALL", 2, "EX-06: mutable-history-arm CONDITIONAL — not attempted this pass"),
    ("self-referential model", 0, "EX-21: plausibly covered by `g_08_running_total_self_ref`, not re-verified against this catalogue id"),
    ("self-referential model", 1, "EX-21: same probe, lateness variant not isolated"),
    ("multi-input column group (merge)", 3, "EX-12: change-feed-postured variant of the merge-degeneration this pass pins for mutable snapshot/unclocked — not separately constructed"),
    ("dedup-to-latest (keyed collapse)", 1, "EX-27: lateness variant of the refusal this pass already pins at column 0"),
    ("dedup-to-latest (keyed collapse)", 4, "EX-27: at-least-once-redelivery variant of the refusal this pass already pins at column 0"),
    ("keyed end-state fold", 1, "EX-24: lateness variant of the HOLDS cell this pass already pins at column 0"),
    ("SCD2 / versioned intervals", 2, "EX-29: REFUSED at the skeleton per the catalogue — the interval row set is a function of the observation sequence, not any replayable input, so `recompute ≡ fold` fails before any op is chosen; gated on the deferred as-of-run contract (OQ2), independent of EX-28's `versioned:` parser gap — not attempted this pass"),
    ("SCD2 / versioned intervals", 3, "EX-28: interval-fold UNSUPPORTED-TODAY per the catalogue — not attempted this pass"),
    ("engine-maintained (MV)", 0, "EX-32: engine-delegation verdict — not attempted this pass"),
    ("engine-maintained (MV)", 2, "EX-32: engine-delegation verdict — not attempted this pass"),
    ("cross-model DAG propagation", 0, "EX-31/EX-33/EX-34: cross-model payload-leak/surrogate-stability/watermark-propagation — graph-layer probes outside this pass's pure-derivation scope"),
    ("cross-model DAG propagation", 1, "EX-34: same gap, lateness variant"),
];

fn matrix_row(name: &str) -> &'static [Cell; 7] {
    &MATRIX
        .iter()
        .find(|(row_name, _)| *row_name == name)
        .unwrap_or_else(|| panic!("KNOWN_GAPS/CLAIMED references unknown matrix row '{name}'"))
        .1
}

#[test]
fn coverage_matrix_is_inhabited() {
    // Every CLAIMED/KNOWN_GAPS entry must point at a real, inhabited cell —
    // catches stale entries (a row rename, a cell that was never really
    // inhabited) that would otherwise silently over-claim coverage.
    for (row, col, note) in CLAIMED.iter().chain(KNOWN_GAPS.iter()) {
        assert!(
            matrix_row(row)[*col].is_some(),
            "'{row}' column {col} is not an inhabited cell in MATRIX (note: {note})"
        );
    }

    // CLAIMED and KNOWN_GAPS must be disjoint — a cell is either proven or
    // named as a gap, never both (that would mask which is actually true).
    for (row, col, _) in CLAIMED {
        assert!(
            !KNOWN_GAPS.iter().any(|(r, c, _)| r == row && c == col),
            "'{row}' column {col} is in both CLAIMED and KNOWN_GAPS"
        );
    }

    // The additive-only property: every inhabited cell of MATRIX is in
    // EXACTLY ONE of CLAIMED/KNOWN_GAPS. A new matrix row (or a new
    // inhabited cell on an existing row) that isn't added to either list
    // fails here — the whole point of this test.
    let mut unaccounted = Vec::new();
    for (row, cells) in MATRIX {
        for (col, cell) in cells.iter().enumerate() {
            let Some(example) = cell else { continue };
            let claimed = CLAIMED.iter().any(|(r, c, _)| r == row && *c == col);
            let gap = KNOWN_GAPS.iter().any(|(r, c, _)| r == row && *c == col);
            if !claimed && !gap {
                unaccounted.push(format!("'{row}' column {col} ({example})"));
            }
        }
    }
    assert!(
        unaccounted.is_empty(),
        "inhabited cell(s) with no CLAIMED or KNOWN_GAPS entry — add a test (CLAIMED) or \
         name the deferral (KNOWN_GAPS) before landing this matrix change:\n{}",
        unaccounted.join("\n")
    );
}

#[test]
fn claimed_and_known_gaps_partition_the_inhabited_cell_count() {
    let inhabited: usize = MATRIX
        .iter()
        .map(|(_, cells)| cells.iter().filter(|c| c.is_some()).count())
        .sum();
    assert_eq!(
        inhabited,
        CLAIMED.len() + KNOWN_GAPS.len(),
        "CLAIMED ({}) + KNOWN_GAPS ({}) must equal the total inhabited-cell count ({}) — \
         a mismatch here (given the disjointness/accounted-for checks above both pass) means \
         a duplicate entry within one of the two lists",
        CLAIMED.len(),
        KNOWN_GAPS.len(),
        inhabited
    );
}
