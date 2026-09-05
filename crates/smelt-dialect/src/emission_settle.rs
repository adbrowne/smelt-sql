//! Compile-path settlement of [`Emission::Conditional`] verdicts.
//!
//! `docs/specs/multi_backend.md` §"Operand-conditional verdicts": arity is
//! read from the source CST, operand class from the same type inference
//! that derives the model's projection, and every conditional entry is
//! resolved to one settled verdict *before* printing. This module is the
//! pure walk that does that resolution; `smelt-dialect` cannot depend on
//! `smelt-db` (layered single-ownership), so the type lookup is a callback
//! supplied by the caller (`smelt-runtime`, which owns a `TypeContext`).
//!
//! The printer never calls [`settle_emissions`] or
//! [`smelt_types::Signature::settle_at`] itself — it consumes the
//! `Vec<(TextRange, SettledEmission)>` this produces, threaded through
//! [`crate::PrintContext::settled_emissions`]. [`settled_verdict_for`] is the
//! printer's one lookup helper: a hit returns the precomputed verdict, a
//! miss (a node `settle_emissions` never visited, or no type context was
//! available at all) falls back to [`smelt_types::CallFacts::unresolved`],
//! which needs only the call's arity — readable from the source CST — and
//! is total.

use smelt_parser::ast::{BinaryExpr, FunctionCall};
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_parser::TextRange;
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, CallFacts, DataType, OperandClass, SettledEmission, Signature};

use crate::position::classify as classify_position;
use crate::PrintContext;
use crate::SqlDialect;

/// The call's argument nodes, in positional order, for either a
/// `FUNCTION_CALL` or a `BINARY_EXPR` operator node. `None` for a node that
/// is neither.
fn call_argument_nodes(node: &SyntaxNode) -> Option<Vec<SyntaxNode>> {
    match node.kind() {
        SyntaxKind::FUNCTION_CALL => {
            let fc = FunctionCall::cast(node.clone())?;
            Some(
                fc.arguments()
                    .into_iter()
                    .map(|e| e.syntax().clone())
                    .collect(),
            )
        }
        SyntaxKind::BINARY_EXPR => {
            let bin = BinaryExpr::cast(node.clone())?;
            let left = bin.left()?;
            let right = bin.right()?;
            Some(vec![left.syntax().clone(), right.syntax().clone()])
        }
        _ => None,
    }
}

/// Build [`CallFacts`] for `node`'s arguments, resolving each argument's
/// [`OperandClass`] via `type_of` (`None` — no inferred type, or `type_of`
/// itself returning `None` — classifies as [`OperandClass::Unresolved`],
/// the same fail-safe direction [`OperandClass::of`] gives `Null`/`Unknown`).
fn call_facts(
    args: &[SyntaxNode],
    type_of: &impl Fn(&SyntaxNode) -> Option<DataType>,
) -> CallFacts {
    let classes = args
        .iter()
        .map(|arg| {
            type_of(arg)
                .map(|dt| OperandClass::of(&dt))
                .unwrap_or(OperandClass::Unresolved)
        })
        .collect();
    CallFacts::new(classes)
}

/// Resolve every registry-backed call/operator in `root` to a settled
/// emission verdict for `dialect`.
///
/// Pure given `type_of` — no I/O, no printing. `type_of` is expected to be a
/// pure lookup into a `TypeContext` built once for the same source tree.
pub fn settle_emissions(
    root: &SyntaxNode,
    dialect: SqlDialect,
    type_of: impl Fn(&SyntaxNode) -> Option<DataType>,
) -> Vec<(TextRange, SettledEmission)> {
    let id = dialect.id();
    root.descendants()
        .filter_map(|node| {
            let (name, position) = match node.kind() {
                SyntaxKind::FUNCTION_CALL => (
                    FunctionCall::cast(node.clone())?.name()?,
                    classify_position(&node, root),
                ),
                SyntaxKind::BINARY_EXPR => {
                    (BinaryExpr::cast(node.clone())?.operator()?, Position::Any)
                }
                _ => return None,
            };
            let sig = BuiltinRegistry::resolve(&name)?;
            let args = call_argument_nodes(&node)?;
            let facts = call_facts(&args, &type_of);
            let verdict = sig.settle_at(id, position, &facts);
            Some((node.text_range(), verdict))
        })
        .collect()
}

/// The printer's one lookup: the precomputed settled verdict for `node` from
/// `ctx.settled_emissions`, or — on a miss — a fallback settlement using
/// only `node`'s arity (readable from the source CST; no type context
/// needed or consulted). Total, and never resolves a class-guarded arm
/// without a real class: an arity-less lookup miss lands on `sig`'s
/// `otherwise` arm just as [`OperandClass::Unresolved`] would.
pub fn settled_verdict_for(
    node: &SyntaxNode,
    sig: &Signature,
    position: Position,
    ctx: &PrintContext,
) -> SettledEmission {
    let range = node.text_range();
    if let Some((_, verdict)) = ctx.settled_emissions.iter().find(|(r, _)| *r == range) {
        return *verdict;
    }
    let arity = call_argument_nodes(node).map(|a| a.len()).unwrap_or(0);
    sig.settle_at(ctx.dialect.id(), position, &CallFacts::unresolved(arity))
}
