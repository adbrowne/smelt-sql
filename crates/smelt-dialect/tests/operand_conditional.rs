//! Compile-path settlement of operand-conditional verdicts
//! (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
//!
//! No production registry entry is `Conditional` yet (phase 7 populates the
//! first ones — `LOG`, `TRUNC`, `TO_JSON`, `//` per class). These tests
//! exercise `settle_emissions`'s walk mechanics — position/arity read off
//! the source CST, operand class read through the caller's `type_of`
//! callback, and the result matching a direct `Signature::settle_at` call —
//! against `//`, the one registered entry whose non-`DuckDB` verdict is
//! already `Unsupported` wholesale. The arm-selection logic itself (first
//! match wins, arity guards, class guards, the `otherwise` fallback) is
//! proven against synthetic signatures in
//! `crates/smelt-types/tests/registry_coverage.rs`.

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
