//! A construct the registry declares `Emission::Unsupported` on a dialect is a
//! compile-time refusal, not a warehouse round trip.
//!
//! The printer still emits such a construct verbatim (it has no diagnostic
//! channel — `print` returns a `String`). `unsupported_emissions` is the pure
//! pre-print check that lets the compile path refuse first.

use smelt_dialect::{unsupported_emissions, SqlDialect};
use smelt_parser::{parse, syntax_kind::SyntaxNode};
use smelt_types::DialectId;

fn tree(sql: &str) -> SyntaxNode {
    parse(sql).syntax()
}

#[test]
fn floor_divide_is_reported_for_bigquery() {
    let found = unsupported_emissions(
        &tree("SELECT a // b AS q FROM t"),
        SqlDialect::BigQuery,
        |_| None,
    );
    assert_eq!(found.len(), 1, "expected exactly one refusal: {found:#?}");
    assert_eq!(found[0].name, "//");
    assert_eq!(found[0].dialect, DialectId::BigQuery);
    assert!(
        found[0].reason.contains("GoogleSQL"),
        "the reason must be the registry's, naming the engine: {}",
        found[0].reason
    );
}

#[test]
fn the_same_model_is_clean_on_duckdb() {
    assert!(
        unsupported_emissions(
            &tree("SELECT a // b AS q FROM t"),
            SqlDialect::DuckDB,
            |_| None
        )
        .is_empty(),
        "`//` is native on DuckDB"
    );
}

#[test]
fn an_unregistered_function_is_not_reported_here() {
    // Recognition is `UnrecognizedFunction`'s job (registry_consistency); this
    // check reports only *declared* refusals, so the two diagnostics cannot
    // double-fire on the same construct.
    assert!(unsupported_emissions(
        &tree("SELECT nonesuch(a) AS q FROM t"),
        SqlDialect::BigQuery,
        |_| None
    )
    .is_empty());
}

#[test]
fn every_occurrence_is_reported_with_its_own_range() {
    let found = unsupported_emissions(
        &tree("SELECT a // b AS q, c // d AS r FROM t"),
        SqlDialect::BigQuery,
        |_| None,
    );
    assert_eq!(found.len(), 2, "{found:#?}");
    assert_ne!(found[0].range, found[1].range);
}

/// The range must cover the offending construct, not the whole statement — a
/// diagnostic anchored to the file start is useless in an editor.
#[test]
fn the_range_covers_the_offending_expression() {
    let sql = "SELECT a // b AS q FROM t";
    let found = unsupported_emissions(&tree(sql), SqlDialect::BigQuery, |_| None);
    let range = found[0].range;
    assert_eq!(&sql[range], "a // b", "range was {range:?}");
}

/// A function-shaped refusal resolves through the same path as an operator one.
/// No entry declares an `Unsupported` function today, so this asserts the shape
/// via the registry rather than pinning a name that may become supported.
#[test]
fn function_and_operator_refusals_share_one_walk() {
    use smelt_types::{BuiltinRegistry, Emission, SyntaxForm};
    let unsupported_calls: Vec<&str> = BuiltinRegistry::names()
        .filter(|n| {
            BuiltinRegistry::resolve(n).is_some_and(|s| {
                s.syntax_form == SyntaxForm::Call
                    && matches!(
                        s.emission_at(DialectId::BigQuery, smelt_types::signatures::Position::Any),
                        Emission::Unsupported { .. }
                    )
            })
        })
        .collect();
    for name in unsupported_calls {
        let sql = format!("SELECT {name}(a) AS q FROM t");
        assert!(
            !unsupported_emissions(&tree(&sql), SqlDialect::BigQuery, |_| None).is_empty(),
            "`{name}` is declared Unsupported on BigQuery but the walk did not report it"
        );
    }
}

/// `PERCENTILE_CONT`/`PERCENTILE_DISC` at a whole-partition window position on
/// BigQuery is `Emission::Rewrite(WithinGroupToAnalytic)`, not
/// `Emission::Unsupported` — but the rewrite still has an admissibility rule
/// of its own (`docs/specs/multi_backend.md` §"Statement-level lowering": a
/// `NULLS FIRST`/`LAST` modifier the analytic form cannot express is refused
/// rather than silently dropped). This must surface here, before the
/// printer ever runs, exactly like a static `Unsupported` verdict.
#[test]
fn a_nulls_modifier_the_analytic_form_cannot_express_is_refused_at_compile_time() {
    let sql = "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x NULLS LAST) \
               OVER (PARTITION BY g) AS med FROM t";
    let found = unsupported_emissions(&tree(sql), SqlDialect::BigQuery, |_| None);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "PERCENTILE_CONT");
    assert!(
        found[0].reason.contains("NULLS"),
        "the reason must name what cannot be expressed: {}",
        found[0].reason
    );
}

/// Phase 8's Spark `Emission::Unsupported` rows closing #178: each carries a
/// reason naming what's missing or why the shape change can't be expressed
/// as a rename or fixed-arity template.
#[test]
fn spark_refuses_glob_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT a GLOB 'a*' FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "GLOB");
    assert!(found[0].reason.contains("GLOB"), "{}", found[0].reason);
}

#[test]
fn spark_refuses_json_array_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT JSON_ARRAY(a, b) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "JSON_ARRAY");
    assert!(
        found[0].reason.contains("json_array"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_json_object_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT JSON_OBJECT('k', a) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "JSON_OBJECT");
    assert!(
        found[0].reason.contains("json_object"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_json_contains_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT JSON_CONTAINS(a, b) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "JSON_CONTAINS");
    assert!(
        found[0].reason.contains("json_contains"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_make_time_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT MAKE_TIME(1, 2, 3) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "MAKE_TIME");
    assert!(found[0].reason.contains("make_time"), "{}", found[0].reason);
}

#[test]
fn spark_refuses_make_timestamptz_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT MAKE_TIMESTAMPTZ(2026, 1, 2, 3, 4, 5) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "MAKE_TIMESTAMPTZ");
    assert!(
        found[0].reason.contains("make_timestamptz"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_quote_ident_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT QUOTE_IDENT(a) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "QUOTE_IDENT");
    assert!(
        found[0].reason.contains("another SQL dialect"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_quote_literal_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT QUOTE_LITERAL(a) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "QUOTE_LITERAL");
    assert!(
        found[0].reason.contains("another SQL dialect"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_truncate_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT TRUNCATE(a, 2) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "TRUNCATE");
    assert!(
        found[0].reason.contains("truncation"),
        "{}",
        found[0].reason
    );
}

#[test]
fn spark_refuses_group_concat_with_a_named_reason() {
    let found = unsupported_emissions(
        &tree("SELECT GROUP_CONCAT(a) AS q FROM t"),
        SqlDialect::SparkSQL,
        |_| None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "GROUP_CONCAT");
    assert!(
        found[0].reason.contains("group_concat"),
        "{}",
        found[0].reason
    );
}

/// The ordinary shape — a plain `ORDER BY` sort key with no `NULLS`
/// modifier — is admissible and reports nothing.
#[test]
fn a_plain_within_group_sort_key_is_not_reported() {
    let sql = "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
               OVER (PARTITION BY g) AS med FROM t";
    assert!(
        unsupported_emissions(&tree(sql), SqlDialect::BigQuery, |_| None).is_empty(),
        "a plain WITHIN GROUP sort key must be admissible"
    );
}
