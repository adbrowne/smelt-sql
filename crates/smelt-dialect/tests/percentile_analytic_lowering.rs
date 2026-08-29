//! `PERCENTILE_CONT`/`PERCENTILE_DISC ... WITHIN GROUP` at a whole-partition
//! window position, on GoogleSQL.
//!
//! GoogleSQL accepts `PERCENTILE_CONT(x, f) OVER (PARTITION BY g)` — the
//! two-argument analytic spelling — but `WITHIN GROUP` under an `OVER`
//! clause is a syntax error there (measured live 2026-08-27, recorded in
//! `docs/specs/multi_backend.md` §"Exact-median lowering"). smelt's
//! ordered-set spelling is rewritten to the analytic spelling in place; the
//! window itself is left untouched, because it is already whole-partition —
//! no CTE is needed.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

fn bigquery(sql: &str) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect: &SqlDialect::BigQuery,
        capabilities: &BackendCapabilities::bigquery(),
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

#[test]
fn whole_partition_within_group_lowers_to_the_analytic_form() {
    let out = bigquery(
        "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS m \
         FROM t",
    );

    assert!(
        !out.to_ascii_uppercase().contains("WITHIN GROUP"),
        "GoogleSQL rejects WITHIN GROUP under an OVER clause — it must not \
         survive into the printed SQL: {out}"
    );
    assert!(
        out.contains("PERCENTILE_CONT(x, 0.5) OVER (PARTITION BY g)"),
        "the sort key becomes the first argument and the window is kept \
         as-is: {out}"
    );
}

#[test]
fn percentile_disc_lowers_the_same_way() {
    let out = bigquery(
        "SELECT PERCENTILE_DISC(0.9) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS m \
         FROM t",
    );
    assert!(
        out.contains("PERCENTILE_DISC(x, 0.9) OVER (PARTITION BY g)"),
        "{out}"
    );
}

#[test]
fn a_descending_sort_key_inverts_the_fraction() {
    let out = bigquery(
        "SELECT PERCENTILE_CONT(0.25) WITHIN GROUP (ORDER BY x DESC) OVER (PARTITION BY g) \
         AS m FROM t",
    );
    assert!(
        out.contains("PERCENTILE_CONT(x, (1 - 0.25)) OVER (PARTITION BY g)"),
        "a DESC sort key inverts the fraction, exactly like AnalyticToCte's \
         aggregate-position lowering: {out}"
    );
}

#[test]
fn other_dialects_keep_within_group_verbatim() {
    for (dialect, caps) in [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        (SqlDialect::SparkSQL, BackendCapabilities::spark()),
        (SqlDialect::PostgreSQL, BackendCapabilities::postgresql()),
    ] {
        let sql = "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) \
                   AS m FROM t";
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
        let out = print(&parsed.syntax(), &ctx);
        assert_eq!(
            out,
            sql,
            "{} must print WITHIN GROUP unchanged",
            dialect.name()
        );
    }
}
