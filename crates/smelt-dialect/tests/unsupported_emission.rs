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
    let found = unsupported_emissions(&tree("SELECT a // b AS q FROM t"), SqlDialect::BigQuery);
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
        unsupported_emissions(&tree("SELECT a // b AS q FROM t"), SqlDialect::DuckDB).is_empty(),
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
        SqlDialect::BigQuery
    )
    .is_empty());
}

#[test]
fn every_occurrence_is_reported_with_its_own_range() {
    let found = unsupported_emissions(
        &tree("SELECT a // b AS q, c // d AS r FROM t"),
        SqlDialect::BigQuery,
    );
    assert_eq!(found.len(), 2, "{found:#?}");
    assert_ne!(found[0].range, found[1].range);
}

/// The range must cover the offending construct, not the whole statement — a
/// diagnostic anchored to the file start is useless in an editor.
#[test]
fn the_range_covers_the_offending_expression() {
    let sql = "SELECT a // b AS q FROM t";
    let found = unsupported_emissions(&tree(sql), SqlDialect::BigQuery);
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
            !unsupported_emissions(&tree(&sql), SqlDialect::BigQuery).is_empty(),
            "`{name}` is declared Unsupported on BigQuery but the walk did not report it"
        );
    }
}
