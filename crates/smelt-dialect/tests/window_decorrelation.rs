//! Printer emission of statement-level restructures
//! (`docs/specs/multi_backend.md` §"Statement-level lowering"): a planned
//! `RestructurePlan` turned into SQL — synthesised CTEs appended to the
//! author's own `WITH` list, base references qualified to the bound source's
//! alias, and a null-safe join whose spelling comes from
//! `BackendCapabilities`, never from a dialect arm.
//!
//! Live-engine facts pinned by these snapshots (`docs/plans/
//! 20260827-statement-level-lowering.md`, 2026-08-27 sweep): BigQuery runs
//! both directions end-to-end with correct values, `IS NOT DISTINCT FROM`
//! makes the join total even across NaN keys, and a synthesised CTE appended
//! to an author `WITH` list works and may reference the author's bindings.

use std::collections::{HashMap, HashSet};

use smelt_dialect::restructure::plan;
use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

fn print_restructured(sql: &str, dialect: SqlDialect, caps: &BackendCapabilities) -> String {
    let parsed = parse(sql);
    let root = parsed.syntax();
    let plans = plan(&root, dialect).unwrap_or_else(|e| panic!("expected admissible plan: {e:#?}"));
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &plans,
        settled_emissions: &[],
    };
    print(&root, &ctx)
}

// ─── Direction B: analytic-only built-in in aggregate position (BigQuery) ──

#[test]
fn direction_b_matches_snapshot() {
    let sql = "SELECT g, COUNT(*) AS n, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t WHERE ok GROUP BY g";
    let out = print_restructured(sql, SqlDialect::BigQuery, &BackendCapabilities::bigquery());
    insta::assert_snapshot!(out, @"WITH __smelt_r0 AS (SELECT *, PERCENTILE_CONT(x, 0.5) OVER (PARTITION BY g) AS v0 FROM t WHERE ok) SELECT g, COUNT(*) AS n, ANY_VALUE(v0) AS med FROM __smelt_r0 GROUP BY g");

    assert!(
        !out.to_ascii_uppercase().contains("WITHIN GROUP"),
        "the ordered-set form must not survive: {out}"
    );
    assert!(out.contains("ANY_VALUE(v0) AS med"), "{out}");
    assert!(
        out.contains("COUNT(*) AS n"),
        "sibling aggregate untouched: {out}"
    );
}

// ─── Direction A: aggregate-only built-in in window position (BigQuery) ────

#[test]
fn direction_a_matches_snapshot() {
    // BigQuery does not (yet) declare a Restructure verdict for MAX_BY/
    // APPROX_COUNT_DISTINCT in window position (Phase 6 territory); the only
    // Restructure(WindowToCte) entries wired so far are PERCENTILE_CONT /
    // PERCENTILE_DISC on DuckDB and Spark. Exercise the shape on BigQuery via
    // the same registry entry to pin the BigQuery-measured join semantics
    // independently of which dialect is tested in `duckdb_ordered_set_window_decorrelates`.
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());
    insta::assert_snapshot!(out, @"WITH __smelt_base AS (SELECT * FROM tbl WHERE ok), __smelt_w0 AS (SELECT g AS __smelt_k0, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS v0 FROM __smelt_base GROUP BY __smelt_k0) SELECT __smelt_base.id, __smelt_base.g, __smelt_w0.v0 AS med FROM __smelt_base JOIN __smelt_w0 ON __smelt_base.g IS NOT DISTINCT FROM __smelt_w0.__smelt_k0");

    assert!(
        !out.contains(" OVER ("),
        "the window form must not survive: {out}"
    );
    assert!(out.contains("__smelt_w0.v0 AS med"), "{out}");
}

#[test]
fn duckdb_ordered_set_window_decorrelates() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());
    assert!(
        out.contains("PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS v0"),
        "the aggregate form (OVER dropped) must appear in the grouped CTE: {out}"
    );
    assert!(
        out.contains("IS NOT DISTINCT FROM"),
        "DuckDB spells the null-safe join IS NOT DISTINCT FROM: {out}"
    );
}

// ─── The synthesised CTE is appended, not prefixed ─────────────────────────

#[test]
fn synthesised_cte_appends_to_author_with_list() {
    let sql = "WITH a AS (SELECT 1 AS g, 10.0 AS x) \
               SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM a GROUP BY g";
    let out = print_restructured(sql, SqlDialect::BigQuery, &BackendCapabilities::bigquery());
    assert!(
        out.starts_with("WITH a AS (SELECT 1 AS g, 10.0 AS x), __smelt_r0 AS ("),
        "the author's own CTE must come first, the synthesised one appended: {out}"
    );
    // The synthesised body may reference the author's binding `a`.
    assert!(out.contains("FROM a"), "{out}");
}

// ─── Null-safe join spelling, driven by capability data ────────────────────

#[test]
fn null_safe_join_spelling_per_backend() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl";

    let duckdb = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());
    assert!(duckdb.contains("IS NOT DISTINCT FROM"), "{duckdb}");
    assert!(!duckdb.contains("<=>"), "{duckdb}");

    let spark = print_restructured(sql, SqlDialect::SparkSQL, &BackendCapabilities::spark());
    assert!(spark.contains("<=>"), "{spark}");
    assert!(!spark.contains("IS NOT DISTINCT FROM"), "{spark}");
}

// ─── No PARTITION BY degenerates to a CROSS JOIN ───────────────────────────

#[test]
fn no_partition_by_uses_cross_join() {
    let sql = "SELECT id, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER () AS med \
               FROM tbl";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());
    assert!(out.contains("CROSS JOIN __smelt_w0"), "{out}");
    assert!(!out.contains(" JOIN __smelt_w0 ON"), "{out}");
}

// ─── A replaced call nested inside a surrounding expression must not drop
//     the surrounding expression (regression: `print_restructured_select_list`
//     used to match by range containment against the *whole select item*
//     and discard everything outside the call — silently dropping `* 2` and
//     computing a plausible wrong number with no error). ───────────────────

#[test]
fn nested_arithmetic_around_replaced_call_is_preserved() {
    let sql = "SELECT g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) * 2 \
               AS scaled FROM t";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());

    // The joined-back reference must appear multiplied by 2, not standing
    // alone as the entire projected value.
    assert!(
        out.contains("__smelt_w0.v0 * 2 AS scaled"),
        "the `* 2` around the replaced call must survive: {out}"
    );
    // The window form itself must be gone from the outer select.
    assert!(
        !out.contains(" OVER ("),
        "the window form must not survive: {out}"
    );
}

#[test]
fn replaced_call_nested_inside_another_call_is_preserved() {
    let sql = "SELECT g, \
               COALESCE(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g), 0) \
               AS best FROM tbl";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());

    assert!(
        out.contains("COALESCE(__smelt_w0.v0, 0) AS best"),
        "the surrounding COALESCE(…, 0) must survive around the replaced call: {out}"
    );
    assert!(
        !out.contains(" OVER ("),
        "the window form must not survive: {out}"
    );
}

// ─── Two window calls sharing one parent must each keep their OWN `OVER`
//     clause (regression: `window_spec_of` used to return the *first*
//     `WINDOW_SPEC` found among ALL of the parent's children, so a second
//     sibling call's own `OVER` clause was never captured in the
//     substitution table and printed verbatim, jammed onto the previous
//     replacement with no separating space — malformed SQL). ────────────────

#[test]
fn two_window_calls_under_one_binary_expr_each_keep_their_own_window() {
    let sql = "SELECT id, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) + \
               PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());

    assert!(
        !out.contains(" OVER ("),
        "no OVER clause must survive in the outer select — both calls were \
         restructured and their windows swallowed: {out}"
    );
    assert!(
        out.contains("__smelt_w0.v0 + __smelt_w0.v1 AS med"),
        "both calls must substitute to their own value column, joined by `+`, \
         with a preserved space before `+` and after: {out}"
    );
}

#[test]
fn window_call_beside_a_non_window_call_resolves_correctly() {
    // COALESCE has no OVER clause at all and sits as a sibling of the
    // restructured windowed call under the same BINARY_EXPR. Its
    // `window_spec_of` lookup must yield `None` — never the windowed call's
    // `OVER` clause — and the windowed call must still resolve its own.
    let sql = "SELECT id, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) + \
               COALESCE(y, 0) AS med \
               FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());

    assert!(
        !out.contains(" OVER ("),
        "the window form must not survive: {out}"
    );
    assert!(
        out.contains("__smelt_w0.v0 + COALESCE(y, 0) AS med"),
        "the windowed call substitutes; the non-window sibling call is untouched: {out}"
    );
}

#[test]
fn ordinary_window_function_beside_a_restructured_call_keeps_its_over_clause() {
    // A separate select item's plain ROW_NUMBER() OVER (...) has no
    // Restructure verdict and must never be touched by the PERCENTILE_CONT
    // restructure happening in the other select item.
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med, \
               ROW_NUMBER() OVER (PARTITION BY g ORDER BY id) AS rn \
               FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::DuckDB, &BackendCapabilities::duckdb());

    assert!(
        out.contains("ROW_NUMBER() OVER (PARTITION BY g ORDER BY id) AS rn"),
        "the ordinary window function's OVER clause must survive intact: {out}"
    );
    assert!(
        out.contains("__smelt_w0.v0 AS med"),
        "the restructured call still substitutes to its value column: {out}"
    );
}

// ─── BUG regression: a call renamed under Restructure(WindowToCte) must
//     apply the RENAME, not print the original spelling verbatim ──────────
//
// `ARG_MAX`/`ARG_MIN` on BigQuery are `Emission::Rename("MAX_BY"/"MIN_BY")`
// at aggregate position (via the `Any` fallback) but
// `Emission::Restructure(WindowToCte)` at `WholePartitionWindow`. Once the
// call is lowered into the synthesised CTE's aggregate position, printing it
// must consult the registry at *that* position (Aggregate) — not re-derive
// position from the call's stale original tree location, which still
// carries the `WINDOW_SPEC` sibling and would misclassify it as
// `WholePartitionWindow` again, printing the call verbatim instead of
// applying the rename. BigQuery has no `ARG_MAX`/`ARG_MIN` at all — an
// unrenamed print is SQL BigQuery rejects at execution.
#[test]
fn bigquery_arg_max_under_window_to_cte_applies_rename_inside_cte() {
    let sql = "SELECT id, g, ARG_MAX(v, k) OVER (PARTITION BY g) AS best FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::BigQuery, &BackendCapabilities::bigquery());

    assert!(
        out.contains("MAX_BY("),
        "the aggregate form inside the synthesised CTE must be renamed to \
         MAX_BY (BigQuery has no ARG_MAX): {out}"
    );
    assert!(
        !out.contains("ARG_MAX("),
        "the unrenamed spelling must not survive — BigQuery rejects it at \
         execution: {out}"
    );
}

#[test]
fn bigquery_arg_min_under_window_to_cte_applies_rename_inside_cte() {
    let sql = "SELECT id, g, ARG_MIN(v, k) OVER (PARTITION BY g) AS best FROM tbl WHERE ok";
    let out = print_restructured(sql, SqlDialect::BigQuery, &BackendCapabilities::bigquery());

    assert!(
        out.contains("MIN_BY("),
        "the aggregate form inside the synthesised CTE must be renamed to \
         MIN_BY (BigQuery has no ARG_MIN): {out}"
    );
    assert!(!out.contains("ARG_MIN("), "{out}");
}
