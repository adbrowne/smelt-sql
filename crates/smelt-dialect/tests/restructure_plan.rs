//! `restructure::plan` — the pure statement-level restructure planner.
//!
//! Correctness oracle: `docs/specs/multi_backend.md` §"Statement-level
//! lowering". Every admissibility-rule test here is written so it FAILS OPEN
//! — i.e. asserts a refusal on a shape that would otherwise plan cleanly —
//! rather than merely checking an error string, so a deleted rule shows up
//! as a newly-green test rather than a silently-still-red one.

use smelt_dialect::restructure::{plan, RestructurePlan};
use smelt_dialect::SqlDialect;
use smelt_parser::{parse, syntax_kind::SyntaxNode};

fn tree(sql: &str) -> SyntaxNode {
    parse(sql).syntax()
}

// ─── The two directions ─────────────────────────────────────────────────────

/// GoogleSQL rejects `PERCENTILE_CONT ... WITHIN GROUP` under `GROUP BY`
/// outright and requires an `OVER` clause instead — the analytic-only-in-
/// aggregate-position shape (`RestructureId::AnalyticToCte`).
#[test]
fn analytic_only_in_aggregate_position_plans_direction_b() {
    let sql = "SELECT g, COUNT(*) AS n, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t WHERE ok GROUP BY g";
    let plans = plan(&tree(sql), SqlDialect::BigQuery).expect("admissible");
    assert_eq!(plans.len(), 1, "{plans:#?}");
    match &plans[0] {
        RestructurePlan::AnalyticToCte {
            source,
            group_keys,
            replacements,
            ..
        } => {
            assert_eq!(source.alias, "__smelt_r0");
            assert!(
                source.where_predicate.is_some(),
                "WHERE must be planted on the bound CTE"
            );
            assert_eq!(group_keys.len(), 1, "{group_keys:#?}");
            assert_eq!(replacements.len(), 1, "{replacements:#?}");
            assert_eq!(replacements[0].value_column, "v0");
            assert!(!replacements[0].fraction_complement);
        }
        other => panic!("expected AnalyticToCte, got {other:#?}"),
    }
}

/// DuckDB and Spark have the ordered-set `PERCENTILE_CONT` aggregate but no
/// window form of it — the aggregate-only-in-window-position shape
/// (`RestructureId::WindowToCte`), admissible only at a whole-partition
/// window.
#[test]
fn aggregate_only_in_whole_partition_window_plans_direction_a() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl WHERE ok";
    let plans = plan(&tree(sql), SqlDialect::DuckDB).expect("admissible");
    assert_eq!(plans.len(), 1, "{plans:#?}");
    match &plans[0] {
        RestructurePlan::WindowToCte { base, groups, .. } => {
            assert_eq!(base.alias, "__smelt_base");
            assert!(
                base.where_predicate.is_some(),
                "WHERE must be planted on the bound source"
            );
            assert_eq!(groups.len(), 1, "{groups:#?}");
            assert_eq!(groups[0].partition_keys.len(), 1);
            assert_eq!(groups[0].calls.len(), 1);
        }
        other => panic!("expected WindowToCte, got {other:#?}"),
    }
}

/// The same shape on Spark, proving the direction is not DuckDB-specific.
#[test]
fn aggregate_only_in_whole_partition_window_plans_on_spark_too() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl";
    let plans = plan(&tree(sql), SqlDialect::SparkSQL).expect("admissible");
    assert_eq!(plans.len(), 1, "{plans:#?}");
    assert!(matches!(plans[0], RestructurePlan::WindowToCte { .. }));
}

/// A running window has no correct CTE form — the registry states this as an
/// ordinary `Position::Window` `Unsupported` verdict, and the planner must
/// surface it rather than plan a wrong-answer lowering. Naming the built-in,
/// the backend, and the whole-partition requirement is the whole point: a
/// caller that only checked `is_err()` could not tell this refusal apart from
/// any other, so the test pins the message content, not just the outcome.
#[test]
fn running_window_is_refused() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g ORDER BY t) \
               AS med FROM tbl";
    let err = plan(&tree(sql), SqlDialect::DuckDB).expect_err("a running window must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert_eq!(err[0].name, "PERCENTILE_CONT");
    assert_eq!(err[0].dialect, smelt_types::DialectId::DuckDb);
    assert!(
        err[0].reason.to_ascii_lowercase().contains("window"),
        "reason must name the whole-partition requirement: {}",
        err[0].reason
    );
}

// ─── Admissibility rule 1: plain GROUP BY only ─────────────────────────────

#[test]
fn rollup_grouping_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t GROUP BY ROLLUP(g)";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("ROLLUP must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("ROLLUP"),
        "{}",
        err[0].reason
    );
}

#[test]
fn cube_grouping_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t GROUP BY CUBE(g)";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("CUBE must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("CUBE"),
        "{}",
        err[0].reason
    );
}

#[test]
fn grouping_sets_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t GROUP BY GROUPING SETS ((g), ())";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("GROUPING SETS must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("GROUPING SETS"),
        "{}",
        err[0].reason
    );
}

/// A plain `GROUP BY` — the same key, no `ROLLUP`/`CUBE`/`GROUPING SETS` —
/// must still plan cleanly. Without this control, the three refusal tests
/// above could pass merely because *any* `GROUP BY` were refused.
#[test]
fn plain_group_by_still_plans() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t GROUP BY g";
    assert!(plan(&tree(sql), SqlDialect::BigQuery).is_ok());
}

// ─── Admissibility rule 2: every occurrence is in the select list ─────────

#[test]
fn occurrence_in_having_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med FROM t \
               GROUP BY g HAVING PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) > 10";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("HAVING occurrence must refuse");
    assert!(!err.is_empty());
    assert!(err
        .iter()
        .any(|e| e.reason.to_ascii_uppercase().contains("HAVING")));
}

#[test]
fn occurrence_in_order_by_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med FROM t \
               GROUP BY g ORDER BY PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x)";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("ORDER BY occurrence must refuse");
    assert!(!err.is_empty());
    assert!(err
        .iter()
        .any(|e| e.reason.to_ascii_uppercase().contains("ORDER BY")));
}

#[test]
fn occurrence_in_qualify_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med FROM t \
               GROUP BY g QUALIFY PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) > 10";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("QUALIFY occurrence must refuse");
    assert!(!err.is_empty());
    assert!(err
        .iter()
        .any(|e| e.reason.to_ascii_uppercase().contains("QUALIFY")));
}

/// Control: a *different* aggregate reused in HAVING is fine — the rule is
/// about the affected built-in reappearing, not about HAVING existing at all.
#[test]
fn a_different_aggregate_in_having_does_not_refuse() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med FROM t \
               GROUP BY g HAVING COUNT(*) > 1";
    assert!(plan(&tree(sql), SqlDialect::BigQuery).is_ok());
}

// ─── Admissibility rule 3: no DISTINCT, no FILTER ──────────────────────────

#[test]
fn distinct_argument_is_refused() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(DISTINCT 0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) \
               AS med FROM tbl";
    let err = plan(&tree(sql), SqlDialect::DuckDB).expect_err("DISTINCT must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("DISTINCT"),
        "{}",
        err[0].reason
    );
}

#[test]
fn filter_clause_is_refused() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) FILTER (WHERE id > 0) \
               OVER (PARTITION BY g) AS med FROM tbl";
    let err = plan(&tree(sql), SqlDialect::DuckDB).expect_err("FILTER must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("FILTER"),
        "{}",
        err[0].reason
    );
}

// ─── Admissibility rule 4: no unexpanded wildcard ──────────────────────────

#[test]
fn unexpanded_wildcard_is_refused() {
    let sql = "SELECT *, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med \
               FROM t GROUP BY g";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("wildcard must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_lowercase().contains("wildcard"),
        "{}",
        err[0].reason
    );
}

// ─── Ordered-set sort key mechanics ─────────────────────────────────────────

#[test]
fn ordered_set_desc_inverts_the_fraction() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x DESC) AS med \
               FROM t GROUP BY g";
    let plans = plan(&tree(sql), SqlDialect::BigQuery).expect("admissible");
    match &plans[0] {
        RestructurePlan::AnalyticToCte { replacements, .. } => {
            assert!(
                replacements[0].fraction_complement,
                "DESC must invert the fraction"
            );
        }
        other => panic!("expected AnalyticToCte, got {other:#?}"),
    }
}

/// Control: `ASC` (the default) must not invert the fraction — otherwise the
/// DESC test above could pass merely because inversion is unconditional.
#[test]
fn ordered_set_asc_does_not_invert_the_fraction() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x ASC) AS med \
               FROM t GROUP BY g";
    let plans = plan(&tree(sql), SqlDialect::BigQuery).expect("admissible");
    match &plans[0] {
        RestructurePlan::AnalyticToCte { replacements, .. } => {
            assert!(!replacements[0].fraction_complement);
        }
        other => panic!("expected AnalyticToCte, got {other:#?}"),
    }
}

#[test]
fn inexpressible_nulls_modifier_is_refused() {
    let sql = "SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x NULLS LAST) AS med \
               FROM t GROUP BY g";
    let err = plan(&tree(sql), SqlDialect::BigQuery).expect_err("NULLS LAST must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_uppercase().contains("NULLS"),
        "{}",
        err[0].reason
    );
}

// ─── WHERE placement ────────────────────────────────────────────────────────

/// `WHERE` must land on the bound source, never on the join — a filtered-out
/// row holding the maximum sort key would otherwise change the answer
/// (`docs/specs/multi_backend.md` §"Statement-level lowering"). Asserted at
/// plan level: the predicate is carried on `WindowToCte::base`, and
/// `GroupBinding` has no predicate field at all for a join condition to hide
/// one in.
#[test]
fn where_is_planned_inside_the_bound_source() {
    let sql = "SELECT id, g, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
               FROM tbl WHERE ok";
    let plans = plan(&tree(sql), SqlDialect::DuckDB).expect("admissible");
    match &plans[0] {
        RestructurePlan::WindowToCte { base, .. } => {
            let pred = base
                .where_predicate
                .as_ref()
                .expect("WHERE must be on the bound source");
            assert_eq!(pred.text().to_string(), "ok");
        }
        other => panic!("expected WindowToCte, got {other:#?}"),
    }
}

// ─── Several windows share one bound source ────────────────────────────────

#[test]
fn several_windows_share_one_bound_source() {
    let sql = "SELECT id, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med_g, \
               PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY h) AS p90_h \
               FROM tbl";
    let plans = plan(&tree(sql), SqlDialect::DuckDB).expect("admissible");
    assert_eq!(plans.len(), 1, "one query block, one plan: {plans:#?}");
    match &plans[0] {
        RestructurePlan::WindowToCte { groups, .. } => {
            assert_eq!(groups.len(), 2, "distinct PARTITION BY keys: {groups:#?}");
            assert_ne!(groups[0].cte_name, groups[1].cte_name);
            assert_eq!(groups[0].calls.len(), 1);
            assert_eq!(groups[1].calls.len(), 1);
        }
        other => panic!("expected WindowToCte, got {other:#?}"),
    }
}

// ─── Non-deterministic partition key ───────────────────────────────────────

#[test]
fn non_deterministic_partition_key_expression_is_refused() {
    let sql = "SELECT id, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY RANDOM()) \
               AS med FROM tbl";
    let err =
        plan(&tree(sql), SqlDialect::DuckDB).expect_err("non-deterministic key must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0]
            .reason
            .to_ascii_lowercase()
            .contains("non-deterministic")
            || err[0]
                .reason
                .to_ascii_lowercase()
                .contains("nondeterministic"),
        "{}",
        err[0].reason
    );
}

// ─── The real-fixture model ─────────────────────────────────────────────────

/// `examples/test_workspace` carries a fixture for each direction, and both
/// must stay diagnostic-free under `example_diagnostics` and plan without
/// refusal here.
#[test]
fn plans_real_example_model() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/test_workspace/models");

    let analytic_sql = std::fs::read_to_string(workspace.join("percentile_by_group.sql")).unwrap();
    let analytic_plans =
        plan(&tree(&analytic_sql), SqlDialect::BigQuery).expect("analytic direction admissible");
    assert_eq!(analytic_plans.len(), 1, "{analytic_plans:#?}");
    assert!(matches!(
        analytic_plans[0],
        RestructurePlan::AnalyticToCte { .. }
    ));

    let window_sql =
        std::fs::read_to_string(workspace.join("percentile_over_partition.sql")).unwrap();
    let window_plans =
        plan(&tree(&window_sql), SqlDialect::DuckDB).expect("window direction admissible");
    assert_eq!(window_plans.len(), 1, "{window_plans:#?}");
    assert!(matches!(
        window_plans[0],
        RestructurePlan::WindowToCte { .. }
    ));
}

// ─── Correlated-subquery refusal ───────────────────────────────────────────

/// A scalar subquery in the select list, correlated against the outer
/// query's table, would otherwise plan cleanly on its own terms (a plain
/// `GROUP BY`, the call only in the select list, no `DISTINCT`/`FILTER`, no
/// wildcard) — the *only* thing that can refuse it is
/// `correlated_block_reason`. This is deliberately not merely an
/// error-string check: dropping that rule turns this from a refusal into a
/// silent `Ok`, which is what "fails open" means here.
#[test]
fn correlated_scalar_subquery_in_select_list_is_refused() {
    let sql = "SELECT outer_t.g, (SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
               FROM inner_t WHERE inner_t.g = outer_t.g GROUP BY inner_t.g) AS med \
               FROM outer_t";
    let err = plan(&tree(sql), SqlDialect::BigQuery)
        .expect_err("a correlated scalar subquery must be refused");
    assert_eq!(err.len(), 1, "{err:#?}");
    assert!(
        err[0].reason.to_ascii_lowercase().contains("correlated"),
        "{}",
        err[0].reason
    );
}

/// Control: the same call, same shape, but hosted in a `FROM`-clause derived
/// table rather than a select-list scalar subquery — not correlated-shaped,
/// and must still plan. Without this control, the refusal test above could
/// pass merely because *every* subquery gets refused, correlated or not.
#[test]
fn non_correlated_subquery_in_from_clause_still_plans() {
    let sql = "SELECT g, med FROM (SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
               AS med FROM inner_t GROUP BY g) sub";
    let plans = plan(&tree(sql), SqlDialect::BigQuery)
        .expect("a FROM-clause derived table is not correlated-shaped");
    assert_eq!(plans.len(), 1, "{plans:#?}");
    assert!(matches!(plans[0], RestructurePlan::AnalyticToCte { .. }));
}
