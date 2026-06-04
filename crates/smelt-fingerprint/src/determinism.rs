//! Inline non-determinism detection.
//!
//! A model whose output is not a pure function of its inputs cannot have its
//! output *proven* equal across versions: re-running it produces different rows,
//! so a fingerprint match is not relation-equality. The reuse layer must treat
//! such a model as non-reusable by default (the §5.5 escape hatches —
//! *accept-current* and *assert-deterministic* — are how an author opts back in;
//! they are a downstream policy, not this detector's concern).
//!
//! Smelt tracks `deterministic` as a declared function/extern property, but
//! non-determinism also enters through **inline SQL with no call node to tag**:
//! a bare `now()`/`random()` in a projection, or a `LIMIT`/`FETCH` that selects
//! *which* rows survive without a provably total order. This detector catches
//! that inline form structurally, independent of any declared property.
//!
//! Soundness direction: the load-bearing guarantee is that anything reported
//! deterministic really is reproducible. Over-reporting non-determinism is safe
//! (the model is rebuilt rather than reused — worst case parity), so every rule
//! here errs toward flagging.

use smelt_parser::ast::{FunctionCall, SelectStmt};
use smelt_parser::syntax_kind::SyntaxKind;

use crate::NonDeterminism;

/// Built-in calls whose result is not a function of their arguments. Matched on
/// the (lowercased) call name, so namespace-qualified spellings resolve to the
/// bare name via [`FunctionCall::name`].
const NON_DET_FUNCTIONS: &[&str] = &[
    // Randomness.
    "random",
    "rand",
    "uuid",
    "gen_random_uuid",
    "uuidv4",
    "uuidv7",
    // Wall-clock / transaction time.
    "now",
    "current_timestamp",
    "current_date",
    "current_time",
    "localtime",
    "localtimestamp",
    "current_localtimestamp",
    "current_localtime",
    "today",
    "get_current_timestamp",
    // Session / transaction identity.
    "txid_current",
    "currval",
    "nextval",
    "version",
    "current_setting",
];

/// Aggregates whose result depends on the order in which input rows are folded.
/// A relation is an unordered multiset, so that order is not fixed; and smelt has
/// no aggregate-`ORDER BY` (or `WITHIN GROUP`) syntax to pin it, so every
/// occurrence is non-deterministic. Order-*insensitive* aggregates (`sum`,
/// `count`, `min`, `max`, `avg`, `bool_and`, …) are pure functions of the input
/// multiset and are deliberately absent.
///
/// `first`/`last` are order-sensitive too, but they are smelt *keywords* (for
/// `NULLS FIRST`/`LAST`), so `first(a)` does not lex as an `IDENT` and cannot be
/// written as an aggregate call today — there is nothing to match. Add them here
/// if smelt ever grants them call syntax.
const ORDER_SENSITIVE_AGGREGATES: &[&str] = &[
    "array_agg",
    "list", // DuckDB alias for array_agg
    "string_agg",
    "group_concat",
    "listagg",
    "any_value",
    "arbitrary", // DuckDB alias for any_value
];

/// Temporal specials that DuckDB also accepts **without** parentheses, so they
/// surface as a bare identifier with no `FUNCTION_CALL` node. Kept to spellings
/// that are unambiguous built-ins (rarely legitimate column names); a false
/// positive here is merely over-conservative.
const PARENLESS_TEMPORAL: &[&str] = &[
    "current_timestamp",
    "current_date",
    "current_time",
    "localtimestamp",
    "localtime",
    "current_localtimestamp",
    "current_localtime",
];

/// Analyse `select` (already function-expanded) for inline non-determinism.
/// Returns one [`NonDeterminism`] per distinct reason found; an empty vector
/// means the model is — as far as the detector can establish — deterministic.
///
/// The walk covers the entire CST of the model, including nested derived tables
/// and CTE bodies, because non-determinism *anywhere* in the expansion taints
/// the model's output.
pub(crate) fn analyze(select: &SelectStmt) -> Vec<NonDeterminism> {
    let root = select.syntax();
    let mut reasons: Vec<String> = Vec::new();

    // 1. Non-deterministic built-in calls (`random()`, `now()`, …).
    for node in root.descendants() {
        if let Some(call) = FunctionCall::cast(node.clone()) {
            if let Some(name) = call.name() {
                let lname = name.to_ascii_lowercase();
                if NON_DET_FUNCTIONS.contains(&lname.as_str()) {
                    push(
                        &mut reasons,
                        format!("non-deterministic built-in `{lname}`"),
                    );
                }
                if ORDER_SENSITIVE_AGGREGATES.contains(&lname.as_str()) {
                    push(
                        &mut reasons,
                        format!("order-sensitive aggregate `{lname}` without a total inner order"),
                    );
                }
            }
        }
    }

    // 2. Bare (parenless) temporal specials — an IDENT that is not a column
    //    qualifier segment (i.e. not immediately after a `.`).
    for tok in root
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::IDENT)
    {
        let lname = tok.text().to_ascii_lowercase();
        if !PARENLESS_TEMPORAL.contains(&lname.as_str()) {
            continue;
        }
        let after_dot = tok
            .prev_token()
            .map(|p| p.kind() == SyntaxKind::DOT)
            .unwrap_or(false);
        if after_dot {
            continue; // a `x.current_date` column reference, not the special
        }
        push(
            &mut reasons,
            format!("non-deterministic temporal special `{lname}`"),
        );
    }

    // 3. Row-slicing tail clauses without a provably total order. `LIMIT`/
    //    `OFFSET`/`FETCH` pick *which* rows survive; the surviving set is
    //    determined only by a total `ORDER BY`, which the detector cannot
    //    cheaply prove (a non-unique sort key still ties). Conservatively, any
    //    such clause anywhere in the model is non-deterministic.
    let has_slice = root.descendants().any(|n| {
        matches!(
            n.kind(),
            SyntaxKind::LIMIT_CLAUSE | SyntaxKind::FETCH_CLAUSE
        )
    });
    if has_slice {
        push(
            &mut reasons,
            "row-slicing clause (LIMIT/OFFSET/FETCH) without a provably total ORDER BY".into(),
        );
    }

    reasons
        .into_iter()
        .map(|reason| NonDeterminism { reason })
        .collect()
}

/// Push `reason` unless an identical one is already recorded, keeping the report
/// free of duplicates (e.g. two `random()` calls produce one reason).
fn push(reasons: &mut Vec<String>, reason: String) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}
