//! `LAG`/`LEAD` window-frame elision on SparkSQL
//! (`docs/specs/multi_backend.md` §"Frame elision on offset functions").
//! Spark refuses any frame on `lag`/`lead`; the SQL standard defines them to
//! ignore it, and DuckDB agrees (measured 2026-09-12 — see the spec section),
//! so dropping the frame changes nothing a caller can observe.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}

fn spark(sql: &str) -> String {
    print_with(sql, &SqlDialect::SparkSQL, &BackendCapabilities::spark())
}

fn duckdb(sql: &str) -> String {
    print_with(sql, &SqlDialect::DuckDB, &BackendCapabilities::duckdb())
}

const SESSIONIZE_LAG: &str = "SELECT LAG(ts_col) OVER (\n\
     PARTITION BY partition_col ORDER BY ts_col\n\
     RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW\n\
     ) AS _prev_ts FROM source";

#[test]
fn lag_prints_without_frame_on_spark() {
    let out = spark(SESSIONIZE_LAG);
    assert!(
        !out.to_ascii_uppercase().contains("RANGE BETWEEN"),
        "Spark refuses any frame on lag — the frame must not survive: {out}"
    );
    assert!(
        out.contains("PARTITION BY partition_col"),
        "the window's PARTITION BY must stay intact: {out}"
    );
    assert!(
        out.contains("ORDER BY ts_col"),
        "the window's ORDER BY must stay intact: {out}"
    );
    assert!(
        out.to_ascii_uppercase().contains("LAG(TS_COL)") || out.contains("LAG(ts_col)"),
        "the call itself must print natively: {out}"
    );
}

#[test]
fn lead_prints_without_frame_on_spark() {
    let sql = "SELECT LEAD(ts_col) OVER (\n\
         PARTITION BY partition_col ORDER BY ts_col\n\
         ROWS BETWEEN 2 PRECEDING AND CURRENT ROW\n\
         ) AS _next_ts FROM source";
    let out = spark(sql);
    assert!(
        !out.to_ascii_uppercase().contains("ROWS BETWEEN"),
        "Spark refuses any frame on lead — the frame must not survive: {out}"
    );
    assert!(out.contains("PARTITION BY partition_col") && out.contains("ORDER BY ts_col"));
}

#[test]
fn duckdb_output_is_unaffected_by_frame_elision() {
    // DuckDB is not the dialect the elision applies to — the source frame
    // must round-trip byte-identically, exactly as every other DuckDB print
    // does (`docs/specs/architecture.md` §"Print-level identity for the
    // DuckDB dialect").
    let out = duckdb(SESSIONIZE_LAG);
    assert!(
        out.contains("RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW"),
        "DuckDB must keep the source frame verbatim: {out}"
    );
}

#[test]
fn frame_on_a_non_offset_window_fn_is_kept_on_spark() {
    // The elision is registry-scoped to LAG/LEAD, not a blanket "drop every
    // frame" rewrite — MAX is a whole-partition/running aggregate for which
    // the frame is load-bearing.
    let sql = "SELECT MAX(v) OVER (\n\
         PARTITION BY g ORDER BY t\n\
         RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW\n\
         ) AS m FROM src";
    let out = spark(sql);
    assert!(
        out.to_ascii_uppercase().contains("RANGE BETWEEN"),
        "a non-offset window function's frame must be kept: {out}"
    );
}

#[test]
fn named_window_lag_keeps_its_frame_on_spark() {
    // A call reached through a named window reference is not eligible for
    // elision (the frame is not the call's own sibling node) — the frame
    // must reach Spark unchanged rather than being silently dropped, so a
    // real refusal from the engine (if any) is the visible failure, never a
    // quietly wrong answer.
    let sql = "SELECT LAG(v) OVER w AS l FROM t \
               WINDOW w AS (PARTITION BY g ORDER BY t ROWS BETWEEN 2 PRECEDING AND CURRENT ROW)";
    let out = spark(sql);
    assert!(
        out.to_ascii_uppercase().contains("ROWS BETWEEN"),
        "a named window's frame must not be silently elided: {out}"
    );
}
