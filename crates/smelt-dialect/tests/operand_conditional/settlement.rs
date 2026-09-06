//! `settle_emissions`/`settled_verdict_for` mechanics, and the `TRUNC`/`TO_JSON`
//! operand-class arms.

use std::collections::{HashMap, HashSet};

use smelt_dialect::emission_settle::settled_verdict_for;
use smelt_dialect::{settle_emissions, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::syntax_kind::SyntaxKind;
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, CallFacts, DataType, DialectId, OperandClass, SettledEmission};

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
