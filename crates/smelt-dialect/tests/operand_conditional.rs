//! Compile-path settlement of operand-conditional verdicts
//! (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
//!
//! Phase 7 populated the first production `Conditional` entries — `LOG`,
//! `TRUNC`, `TO_JSON`, `//` per class — on Spark. These tests exercise
//! `settle_emissions`'s walk mechanics — position/arity read off the source
//! CST, operand class read through the caller's `type_of` callback, and the
//! result matching a direct `Signature::settle_at` call — against `//`, and
//! (below) the first-argument-class arms of `TRUNC`/`TO_JSON` and the
//! non-`Conditional` `DAYOFWEEK` template. The arm-selection logic itself
//! (first match wins, arity guards, class guards, the `otherwise` fallback)
//! is proven against synthetic signatures in
//! `crates/smelt-types/tests/registry_coverage.rs`.

use std::collections::{HashMap, HashSet};

use smelt_dialect::emission_settle::settled_verdict_for;
use smelt_dialect::{print, settle_emissions, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::syntax_kind::SyntaxKind;
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, CallFacts, DataType, DialectId, OperandClass, SettledEmission};

fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities) -> String {
    let parsed = smelt_parser::parse(sql);
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

fn floor_divide_root() -> smelt_parser::syntax_kind::SyntaxNode {
    smelt_parser::parse("SELECT a // b FROM t").syntax()
}

#[test]
fn settle_emissions_resolves_a_call_from_its_operand_types() {
    let root = floor_divide_root();
    let settled = settle_emissions(&root, SqlDialect::SparkSQL, |_node| Some(DataType::Integer));
    assert_eq!(
        settled.len(),
        1,
        "expected exactly one registered call/operator: {settled:?}"
    );

    let sig = BuiltinRegistry::resolve("//").expect("`//` is registered");
    let expected = sig.settle_at(
        DialectId::SparkSql,
        Position::Any,
        &CallFacts::new(vec![OperandClass::Integral, OperandClass::Integral]),
    );
    assert_eq!(settled[0].1, expected);
}

#[test]
fn settle_emissions_takes_otherwise_when_an_operand_type_is_unresolved() {
    let root = floor_divide_root();
    let settled = settle_emissions(&root, SqlDialect::SparkSQL, |_node| None);
    assert_eq!(settled.len(), 1);

    let sig = BuiltinRegistry::resolve("//").expect("`//` is registered");
    let expected = sig.settle_at(
        DialectId::SparkSql,
        Position::Any,
        &CallFacts::unresolved(2),
    );
    assert_eq!(settled[0].1, expected);
}

#[test]
fn a_settled_verdict_reaches_the_printer_by_range() {
    let root = floor_divide_root();
    let node = root
        .descendants()
        .find(|n| n.kind() == SyntaxKind::BINARY_EXPR)
        .expect("expected a BINARY_EXPR for `a // b`");
    let sig = BuiltinRegistry::resolve("//").expect("`//` is registered");

    // A precomputed verdict that direct settlement would *not* produce
    // (`//` on DuckDB is `Native`) — proving the printer's lookup returns
    // the precomputed value by range rather than re-resolving the arm
    // itself.
    let precomputed = vec![(node.text_range(), SettledEmission::Rename("PRECOMPUTED"))];
    let dialect = SqlDialect::DuckDB;
    let capabilities = BackendCapabilities::duckdb();
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: &capabilities,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &precomputed,
    };
    assert_eq!(
        settled_verdict_for(&node, sig, Position::Any, &ctx),
        SettledEmission::Rename("PRECOMPUTED")
    );

    // A lookup miss (no precomputed entry) falls back to an arity-only
    // settlement, which for `//` on DuckDB really is `Native`.
    let ctx_miss = PrintContext {
        settled_emissions: &[],
        ..ctx
    };
    assert_eq!(
        settled_verdict_for(&node, sig, Position::Any, &ctx_miss),
        SettledEmission::Native
    );
}

#[test]
fn dayofweek_prints_the_shift_template_on_spark() {
    let sql = "SELECT DAYOFWEEK(d) FROM t";
    assert_eq!(
        print_with(sql, &SqlDialect::SparkSQL, &BackendCapabilities::spark()),
        "SELECT (DAYOFWEEK(d) - 1) FROM t",
        "the non-call template's whole output must be parenthesised"
    );
    assert_eq!(
        print_with(sql, &SqlDialect::DuckDB, &BackendCapabilities::duckdb()),
        "SELECT DAYOFWEEK(d) FROM t"
    );
}

/// Phase 8's Spark `Emission::Template` rows — `AGE`, `DATE_SUB`,
/// `TO_SECONDS` — closing #178. Each is call-shaped, so its own arguments
/// substitute bare (no extra wrapping); `AGE`/`DATE_SUB` land on a
/// `BINARY_EXPR`-equivalent shape and their whole non-call output is
/// parenthesised, matching `DAYOFWEEK`'s precedent above.
#[test]
fn age_prints_the_spark_form() {
    assert_eq!(
        print_with(
            "SELECT AGE(a, b) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT (a - b) FROM t"
    );
}

#[test]
fn date_sub_prints_the_spark_form() {
    // Phase 9: the bare infix form reports `DATE` on Spark, not smelt's
    // declared `Timestamp` return type, so the template carries an explicit
    // cast (docs/outcomes/20260904-dialect-emission-vocabulary phase 9).
    assert_eq!(
        print_with(
            "SELECT DATE_SUB(a, b) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT CAST(a - b AS TIMESTAMP) FROM t"
    );
}

#[test]
fn to_seconds_prints_the_spark_form() {
    assert_eq!(
        print_with(
            "SELECT TO_SECONDS(a) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT make_interval(0, 0, 0, 0, 0, 0, a) FROM t"
    );
}

#[test]
fn trunc_and_to_json_settle_by_first_argument_class() {
    let trunc_sig = BuiltinRegistry::resolve("TRUNC").expect("TRUNC is registered");
    assert_eq!(
        trunc_sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Temporal, OperandClass::String])
        ),
        SettledEmission::Native
    );
    assert_eq!(
        trunc_sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Decimal])
        ),
        SettledEmission::Unsupported {
            reason: "Spark's TRUNC is temporal-only; there is no numeric TRUNC"
        }
    );

    let to_json_sig = BuiltinRegistry::resolve("TO_JSON").expect("TO_JSON is registered");
    assert_eq!(
        to_json_sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Composite])
        ),
        SettledEmission::Native
    );
    assert_eq!(
        to_json_sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Integral])
        ),
        SettledEmission::Unsupported {
            reason: "Spark's TO_JSON requires a struct, array or map argument; there is no \
                     scalar TO_JSON"
        }
    );
}
