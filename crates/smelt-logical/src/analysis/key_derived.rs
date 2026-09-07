//! Key-derived-expression proof for key temporal locality route 2
//! (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
//! time-partitioned output)", route 2's **derived** sub-route).
//!
//! A partition projection is a per-key constant, with no declaration
//! needed, when every column it references is itself a `unique_key` column
//! and the expression contains no run-/row-nondeterministic function call —
//! a `unique_key` column never changes across merges, so a deterministic
//! function of key columns alone is a per-key constant by the same
//! argument `rules::cumulative::classify_once_write`'s bare key-derived
//! spelling already makes, extended here to an arbitrary deterministic
//! wrapper (a `CAST`, a `MIN`/`MAX` reduction, …) rather than only a bare
//! column reference.
//!
//! Leaf classifier (`docs/specs/architecture.md` §"Property composition
//! walk rule"): operates over the model's own select list via
//! [`crate::analysis::analyze_select`], never re-scanning raw SQL text and
//! never composing across nodes. [`crate::maintenance::locality::
//! establish_locality`] is the sole caller.

use crate::analysis::expr_util::collect_column_refs;
use crate::analysis::monotonicity::{classify_function_determinism, FunctionDeterminism};
use crate::analysis::{analyze_select, item_alias, item_expr};

/// Whether a model's partition projection is a **derived** key-determined
/// column: [`Derived`](KeyDerivedVerdict::Derived) when every column
/// reference in its expression is a `unique_key` column and the expression
/// contains no nondeterministic function call;
/// [`NotDerived`](KeyDerivedVerdict::NotDerived) otherwise, naming the
/// reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyDerivedVerdict {
    /// The projection is a deterministic function of `unique_key` columns
    /// alone — a per-key constant, no declaration needed.
    Derived,
    /// Not provably key-derived; names the reason (an offending column
    /// reference, a nondeterministic function, or an absent projection).
    NotDerived(String),
}

/// Derive [`KeyDerivedVerdict`] for `partition_column` in `sql`'s outer
/// SELECT.
///
/// `unique_key` and `partition_column` matching is case-insensitive,
/// consistent with [`crate::analysis::not_null::column_provably_not_null`].
pub fn key_derived_partition_verdict(
    sql: &str,
    unique_key: &[String],
    partition_column: &str,
) -> KeyDerivedVerdict {
    let Some(analysis) = analyze_select(sql) else {
        return KeyDerivedVerdict::NotDerived(
            "the model's SQL could not be classified for the key-derived proof".to_string(),
        );
    };
    let Some(item) = analysis
        .items
        .iter()
        .find(|item| item_alias(item).eq_ignore_ascii_case(partition_column))
    else {
        return KeyDerivedVerdict::NotDerived(format!(
            "`{partition_column}` is not a projection in the model's own select list"
        ));
    };
    let expr = item_expr(item);

    for cref in collect_column_refs(expr) {
        let name = cref.name();
        if !unique_key.iter().any(|k| k.eq_ignore_ascii_case(name)) {
            return KeyDerivedVerdict::NotDerived(format!(
                "the expression references `{name}`, which is not a `unique_key` column"
            ));
        }
    }

    if let Some(name) = own_function_call_names(expr).into_iter().find(|name| {
        !matches!(
            classify_function_determinism(name),
            FunctionDeterminism::Neither
        )
    }) {
        return KeyDerivedVerdict::NotDerived(format!(
            "the expression calls `{name}`, a run-/row-nondeterministic function"
        ));
    }

    KeyDerivedVerdict::Derived
}

/// Function-call names in `expr`'s own text, not descending into a nested
/// `SUBQUERY` node. A local copy of `analysis::walk::own_function_call_names`
/// (`pub(crate)` to that module, not exported) — this leaf classifier does
/// not participate in the shared composition walk, so it needs no access to
/// `walk`'s own scope machinery, only the same subquery-exclusion rule.
fn own_function_call_names(expr: &smelt_parser::Expr) -> Vec<String> {
    fn collect(node: &smelt_parser::syntax_kind::SyntaxNode, out: &mut Vec<String>) {
        if node.kind() == smelt_parser::SyntaxKind::SUBQUERY {
            return;
        }
        if node.kind() == smelt_parser::SyntaxKind::FUNCTION_CALL {
            if let Some(func) = smelt_parser::FunctionCall::cast(node.clone()) {
                if let Some(name) = func.name() {
                    out.push(name);
                }
            }
        }
        for child in node.children() {
            collect(&child, out);
        }
    }
    let mut out = Vec::new();
    collect(expr.syntax(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(cols: &[&str]) -> Vec<String> {
        cols.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cast_of_a_key_column_is_key_derived() {
        let sql = "SELECT id, d, CAST(d AS DATE) AS pdate, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id, d";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        assert_eq!(verdict, KeyDerivedVerdict::Derived);
    }

    #[test]
    fn min_max_wrapper_over_a_key_column_is_key_derived() {
        let sql = "SELECT id, MAX(d) AS pdate, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        assert_eq!(verdict, KeyDerivedVerdict::Derived);
    }

    #[test]
    fn reference_outside_the_key_is_not_derived() {
        let sql = "SELECT id, d, other, CAST(other AS DATE) AS pdate, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id, d, other";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        match verdict {
            KeyDerivedVerdict::NotDerived(reason) => {
                assert!(
                    reason.contains("other"),
                    "reason must name `other`: {reason}"
                );
            }
            other => panic!("expected NotDerived, got {other:?}"),
        }
    }

    #[test]
    fn mixed_key_and_non_key_refs_are_not_derived() {
        let sql = "SELECT id, d, other, (d + other) AS pdate, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id, d, other";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        assert!(matches!(verdict, KeyDerivedVerdict::NotDerived(_)));
    }

    #[test]
    fn nondeterministic_function_is_not_derived() {
        let sql = "SELECT id, d, CAST(NOW() AS DATE) AS pdate, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id, d";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        match verdict {
            KeyDerivedVerdict::NotDerived(reason) => {
                assert!(
                    reason.to_uppercase().contains("NOW"),
                    "reason must name the nondeterministic function: {reason}"
                );
            }
            other => panic!("expected NotDerived, got {other:?}"),
        }
    }

    #[test]
    fn absent_projection_is_not_derived() {
        let sql = "SELECT id, d, COUNT(*) AS n \
                   FROM smelt.sources.raw.events GROUP BY id, d";
        let verdict = key_derived_partition_verdict(sql, &key(&["id", "d"]), "pdate");
        match verdict {
            KeyDerivedVerdict::NotDerived(reason) => {
                assert!(
                    reason.contains("pdate"),
                    "reason must name the missing projection: {reason}"
                );
            }
            other => panic!("expected NotDerived, got {other:?}"),
        }
    }
}
