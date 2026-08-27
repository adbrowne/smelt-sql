//! Pre-print refusal of constructs the registry declares unsupported.
//!
//! The printer has no diagnostic channel — `print` returns a plain `String`, so
//! a construct the target dialect cannot express would otherwise reach the
//! warehouse and fail there. This module is the pure check the compile path
//! runs *before* printing, turning an engine-side error into a compile-time
//! diagnostic that names both the construct and the backend.
//!
//! Single ownership holds: the verdict is `BuiltinRegistry` data
//! (`Signature::emission_at`), never a list restated here.

use smelt_parser::ast::BinaryExpr;
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_parser::FunctionCall;
use smelt_parser::{TextRange, TextSize};
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, DialectId, Emission};

use crate::position::classify as classify_position;
use crate::SqlDialect;

/// One construct the target dialect cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedEmission {
    /// The canonical registry name (`"//"`, `"REGEXP_MATCHES"`, …), not the
    /// author's spelling — the diagnostic names what the registry refused.
    pub name: &'static str,
    /// The dialect that cannot express it.
    pub dialect: DialectId,
    /// The registry's reason, verbatim. Written for a user, not a maintainer.
    pub reason: &'static str,
    /// The offending expression's span in the source text.
    ///
    /// A `TextRange`, per the diagnostic range-encoding invariant — conversion
    /// to (line, column) happens once, at the diagnostic boundary.
    pub range: TextRange,
}

/// The node's span with trailing trivia removed.
///
/// A `BINARY_EXPR`'s Rowan range absorbs the whitespace that follows it, so an
/// untrimmed range underlines `a // b ` — one column too wide in an editor.
fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let range = node.text_range();
    let text = node.text().to_string();
    let trailing = text.len() - text.trim_end().len();
    TextRange::new(range.start(), range.end() - TextSize::from(trailing as u32))
}

/// Walk `root` for constructs the registry declares unsupported on `dialect`.
///
/// Pure: no I/O, no printing. A name absent from the registry is *not* reported
/// here — unrecognised functions are `UnrecognizedFunction`'s business, so the
/// two diagnostics cannot double-fire on one construct.
pub fn unsupported_emissions(root: &SyntaxNode, dialect: SqlDialect) -> Vec<UnsupportedEmission> {
    let id = dialect.id();
    root.descendants()
        .filter_map(|node| {
            let (name, position) = match node.kind() {
                SyntaxKind::FUNCTION_CALL => (
                    FunctionCall::cast(node.clone())?.name()?,
                    classify_position(&node, root),
                ),
                // Operators are never a call in window/aggregate position —
                // their verdicts are stated with `Position::Any`.
                SyntaxKind::BINARY_EXPR => {
                    (BinaryExpr::cast(node.clone())?.operator()?, Position::Any)
                }
                _ => return None,
            };
            let sig = BuiltinRegistry::resolve(&name)?;
            match sig.emission_at(id, position) {
                Emission::Unsupported { reason } => Some(UnsupportedEmission {
                    name: sig.name.as_str(),
                    dialect: id,
                    reason,
                    range: trimmed_range(&node),
                }),
                _ => None,
            }
        })
        .collect()
}
