//! Native pipe emission — the `supports_pipe_syntax = true` side of the flag.
//!
//! `pipe_lowering.rs` covers the `false` side: every other backend rewrites a
//! `PIPE_QUERY` to standard SQL. BigQuery is the only backend advertising
//! native support, so it is the only one whose printer emits `|>` verbatim.
//!
//! This is the offline half of the pair that keeps that flag honest. The live
//! half is `smelt-cli`'s `pipe_parity`, which runs the *same* query against a
//! real GoogleSQL warehouse and asserts it computes the same relation DuckDB's
//! lowered form does. Neither half is sufficient alone: this test proves the
//! warehouse is sent pipes (the parity leg would still pass on lowered SQL,
//! which BigQuery also accepts), and the parity leg proves the pipes it is sent
//! are accepted and mean the same thing.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

/// Byte-identical to `PIPE_MODEL` in `crates/smelt-cli/tests/pipe_parity.rs`,
/// minus the `smelt.` reference prefix the runtime rewrites — so the form this
/// test pins is the form that suite executes live.
const PIPE_SQL: &str = "\
FROM pipe_base
|> WHERE val >= 50
|> EXTEND val * 2 AS double_val
|> AGGREGATE SUM(double_val) AS total_double, COUNT(*) AS n GROUP BY grp
|> WHERE total_double > 100
|> ORDER BY grp
|> LIMIT 10";

fn print_with(sql: &str, dialect: SqlDialect, caps: BackendCapabilities) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: &caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
    };
    print(&parsed.syntax(), &ctx)
}

/// BigQuery emits the pipe query as pipes — every stage, in source order —
/// rather than lowering it.
///
/// **Red if `supports_pipe_syntax` is flipped to `false` for BigQuery**, which
/// is what makes `pipe_parity`'s BigQuery leg meaningful: without this test,
/// that leg would keep passing on lowered SQL and prove nothing about the
/// native path.
#[test]
fn bigquery_emits_pipes_natively() {
    let out = print_with(
        PIPE_SQL,
        SqlDialect::BigQuery,
        BackendCapabilities::bigquery(),
    );

    assert!(
        out.contains("|>"),
        "BigQuery advertises supports_pipe_syntax = true and must emit pipes, got: {out}"
    );
    for stage in [
        "|> WHERE val >= 50",
        "|> EXTEND val * 2 AS double_val",
        "|> AGGREGATE SUM(double_val) AS total_double, COUNT(*) AS n GROUP BY grp",
        "|> WHERE total_double > 100",
        "|> ORDER BY grp",
        "|> LIMIT 10",
    ] {
        assert!(
            out.contains(stage),
            "expected stage `{stage}` to survive verbatim, got: {out}"
        );
    }
    // Native emission is passthrough, not a rewrite: no lowered artefact.
    assert!(
        !out.contains("GROUP BY grp HAVING"),
        "native emission must not lower the post-aggregate WHERE to HAVING, got: {out}"
    );
}

/// The same query on the lowering backends carries no `|>` — the pairing that
/// makes the BigQuery assertion above a difference rather than a tautology.
#[test]
fn lowering_backends_emit_no_pipes_for_the_same_query() {
    for (dialect, caps, label) in [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb(), "DuckDB"),
        (
            SqlDialect::SparkSQL,
            BackendCapabilities::spark_delta(),
            "Spark(Delta)",
        ),
    ] {
        let out = print_with(PIPE_SQL, dialect, caps);
        assert!(!out.contains("|>"), "{label} must not emit |>, got: {out}");
    }
}
