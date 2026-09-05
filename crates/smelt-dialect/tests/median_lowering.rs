//! GoogleSQL has no `MEDIAN`. These tests pin the exact lowerings measured
//! against the live warehouse by `scripts/bigquery-probe-lowering.sh`
//! (recorded in `docs/research/20260816-bigquery-backend.md`):
//!
//! - aggregate position (`GROUP BY`): an `ARRAY_AGG`-indexing expression, the
//!   only *exact* form GoogleSQL accepts where an aggregate is required —
//!   `PERCENTILE_CONT` is analytic-only and `APPROX_QUANTILES` is approximate.
//! - window position (`OVER`): `PERCENTILE_CONT(x, 0.5)`, which is exact and
//!   interpolating, matching DuckDB's `MEDIAN` on an even-count fixture.
//!
//! Every other dialect keeps `MEDIAN` verbatim.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;
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

fn bigquery(sql: &str) -> String {
    print_with(sql, &SqlDialect::BigQuery, &BackendCapabilities::bigquery())
}

#[test]
fn bigquery_lowers_grouped_median_to_the_exact_array_agg_form() {
    let out = bigquery("SELECT d, MEDIAN(val) AS med_val FROM events GROUP BY d");

    assert!(
        !out.to_ascii_uppercase().contains("MEDIAN("),
        "GoogleSQL has no MEDIAN — it must not survive into the printed SQL: {out}"
    );
    assert!(
        out.contains("ARRAY_AGG(val IGNORE NULLS ORDER BY val)"),
        "the measured exact lowering aggregates the argument into a sorted, \
         NULL-free array: {out}"
    );
    // The even-count branch averages the two middle elements; the odd-count
    // branch takes the single middle one. Both indices are present.
    assert!(
        out.contains(
            "SAFE_OFFSET(DIV(ARRAY_LENGTH(ARRAY_AGG(val IGNORE NULLS ORDER BY val)), 2) - 1)"
        ),
        "the even-count branch must index the lower middle element: {out}"
    );
    assert!(
        out.contains("MOD(ARRAY_LENGTH(ARRAY_AGG(val IGNORE NULLS ORDER BY val)), 2) = 1"),
        "parity of the element count must select the branch: {out}"
    );
    // The rest of the statement is untouched.
    assert!(
        out.contains("SELECT d,") && out.contains("FROM events GROUP BY d"),
        "{out}"
    );
}

#[test]
fn bigquery_lowers_windowed_median_to_percentile_cont() {
    let out = bigquery("SELECT MEDIAN(val) OVER (PARTITION BY id) AS m FROM events");

    assert!(
        !out.to_ascii_uppercase().contains("MEDIAN("),
        "GoogleSQL has no MEDIAN in window position either: {out}"
    );
    assert!(
        out.contains("PERCENTILE_CONT(val, 0.5) OVER (PARTITION BY id)"),
        "window-position MEDIAN lowers to the exact analytic form: {out}"
    );
}

#[test]
fn bigquery_lowers_median_over_an_expression_argument() {
    let out = bigquery("SELECT MEDIAN(a + b) AS m FROM events GROUP BY d");
    assert!(
        out.contains("ARRAY_AGG(a + b IGNORE NULLS ORDER BY a + b)"),
        "the argument expression, not just a bare column, is aggregated: {out}"
    );
}

/// The compiler's type-cast wrapper re-parses printed SQL to name its columns
/// (`smelt_runtime::compile::SqlCompiler::apply_type_casts`), so a lowering that
/// smelt's own parser cannot read back silently loses the alias and the wrapper
/// invents a positional `_col2` the backend does not have. Both lowerings must
/// therefore round-trip: `IF(...)` did not (smelt reserves `IF` for its own
/// conditional), which is why the aggregate form is spelled `CASE WHEN`.
#[test]
fn the_lowered_sql_parses_back_with_its_alias_intact() {
    for sql in [
        "SELECT d, MEDIAN(val) AS med_val FROM events GROUP BY d",
        "SELECT MEDIAN(val) OVER (PARTITION BY id) AS med_val FROM events",
    ] {
        let out = bigquery(sql);
        let parsed = parse(&out);
        assert!(
            parsed.errors.is_empty(),
            "{out}\nmust parse back cleanly, got: {:?}",
            parsed.errors
        );

        let file = File::cast(parsed.syntax()).expect("file");
        let select = file.select_stmt().expect("select");
        let aliases: Vec<_> = select
            .select_list()
            .expect("select list")
            .items()
            .filter_map(|item| item.alias())
            .collect();
        assert!(
            aliases.iter().any(|a| a == "med_val"),
            "{out}\nmust keep its `med_val` alias, got aliases {aliases:?}"
        );
    }
}

#[test]
fn other_dialects_keep_median_verbatim() {
    for (dialect, caps) in [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        (SqlDialect::SparkSQL, BackendCapabilities::spark()),
    ] {
        let sql = "SELECT d, MEDIAN(val) AS med_val FROM events GROUP BY d";
        let out = print_with(sql, &dialect, &caps);
        assert_eq!(out, sql, "{} must print MEDIAN unchanged", dialect.name());
    }
}
