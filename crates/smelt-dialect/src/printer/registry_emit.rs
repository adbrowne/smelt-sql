//! Registry-driven emission: a call's settled `Emission` verdict rendered
//! as a rename, a template, or a `RewriteId` dispatch. Every per-dialect
//! spelling here is registry data, never a name match or a dialect arm.

use smelt_parser::ast::BinaryExpr;
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::FunctionCall;

use crate::emission_settle::settled_verdict_for;
use crate::position::classify as classify_position;
use smelt_types::signatures::Position;
use smelt_types::{is_call_shaped_template, BuiltinRegistry, RewriteId, SettledEmission};

use super::print_node;
use super::restructure_emit::active_position_override_for;
use super::restructure_emit::print_trimmed;
use super::PrintContext;

/// Lower `MEDIAN(x)` to GoogleSQL, which has no `MEDIAN` built-in.
///
/// Returns `true` when the call was printed here, `false` to fall through to
/// the normal path (leaving the call verbatim so the engine rejects it loudly
/// rather than smelt guessing).
///
/// Both forms below are **exact**, which is the whole constraint: substituting
/// an approximate median under the equivalence oracle would make it report
/// divergences that are artefacts of the substitution, or hide real ones.
/// `APPROX_QUANTILES` is an aggregate but approximate, so it is not a candidate.
/// Measured against the live warehouse by `scripts/bigquery-probe-lowering.sh`
/// on the fixture `val ∈ {1,2,3,4}` — an even count, where an interpolating
/// median (2.5) and a nearest-rank one (2) differ:
///
/// - **Window position** (`MEDIAN(x) OVER w`) → `PERCENTILE_CONT(x, 0.5) OVER w`.
///   Exact and interpolating; measured 2.5, agreeing with DuckDB's `MEDIAN`.
///   `PERCENTILE_DISC` returns 2 and is therefore *not* the equivalent.
/// - **Aggregate position** (`SELECT MEDIAN(x) … GROUP BY …`) →
///   an `ARRAY_AGG`-indexing expression. `PERCENTILE_CONT` is analytic-only in
///   GoogleSQL and is rejected outright in a `GROUP BY` query, so it cannot
///   stand here. Binding the array to a name via `UNNEST([ARRAY_AGG(…)])` is
///   also rejected (`Aggregate function ARRAY_AGG not allowed in UNNEST`), which
///   is why the sub-expression is repeated rather than named. Measured on
///   grouped fixtures against DuckDB's answers: even count → 2.5, odd → 2,
///   NULLs ignored → 1.5, all-NULL group → NULL.
///
/// The aggregate form casts to `FLOAT64`, matching DuckDB's `MEDIAN` return
/// type for numeric input. A temporal argument — which DuckDB's `MEDIAN` also
/// accepts — makes BigQuery reject the cast, i.e. it fails loud rather than
/// returning a wrong value.
fn print_bigquery_median(
    node: &SyntaxNode,
    fc: &FunctionCall,
    position: Position,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let args = fc.arguments();
    let [arg] = args.as_slice() else {
        return false;
    };
    let mut arg_sql = String::new();
    print_node(arg.syntax(), ctx, &mut arg_sql);
    let arg_sql = arg_sql.trim();
    if arg_sql.is_empty() {
        return false;
    }

    // The call's position is decided once, by the compile path, from the
    // source CST (`position::classify`) — this function is never handed
    // anything else to derive it from.
    let windowed = matches!(position, Position::WholePartitionWindow | Position::Window);
    if windowed {
        out.push_str(&format!("PERCENTILE_CONT({arg_sql}, 0.5)"));
        push_trailing_trivia(node, out);
        return true;
    }

    let sorted = format!("ARRAY_AGG({arg_sql} IGNORE NULLS ORDER BY {arg_sql})");
    let mid = format!("DIV(ARRAY_LENGTH({sorted}), 2)");
    let at = |index: &str| format!("CAST({sorted}[SAFE_OFFSET({index})] AS FLOAT64)");
    out.push_str(&format!(
        "CASE WHEN MOD(ARRAY_LENGTH({sorted}), 2) = 1 THEN {upper} \
         ELSE ({lower} + {upper}) / 2 END",
        upper = at(&mid),
        lower = at(&format!("{mid} - 1")),
    ));
    push_trailing_trivia(node, out);
    true
}

/// Is `node`'s printed text an atom (needs no wrapping when substituted into a
/// non-call template), or compound (needs wrapping so the template's
/// surrounding text can't misparse it)?
///
/// Peels exactly the transparent `EXPRESSION` wrapper the parser puts around
/// every function argument and every parenthesised group (`parse_expression`
/// always opens one) down to the node it actually wraps — an `EXPRESSION`
/// wrapping a bare token (identifier, literal) has no node children and the
/// loop stops immediately, classifying it as an atom. What remains is
/// compound exactly when it is `BINARY_EXPR` (which also covers comparisons
/// and unary forms — `is_unary()` is `true` when `right()` is `None`),
/// `CASE_EXPR`, or `CAST_EXPR`; anything else (a literal, an identifier, a
/// column reference, a `FUNCTION_CALL`) is an atom.
fn is_compound_argument(node: &SyntaxNode) -> bool {
    let mut inner = node.clone();
    while inner.kind() == SyntaxKind::EXPRESSION {
        let mut children = inner.children();
        match (children.next(), children.next()) {
            (Some(only), None) => inner = only,
            _ => break,
        }
    }
    matches!(
        inner.kind(),
        SyntaxKind::BINARY_EXPR | SyntaxKind::CASE_EXPR | SyntaxKind::CAST_EXPR
    )
}

/// Interpret an `Emission::Template` string against `node`'s positional
/// arguments (`docs/specs/multi_backend.md` §"Template emission"). The one
/// generic routine every `Emission::Template` row is printed through — it
/// matches no function name and reads no template text to decide behaviour
/// beyond substituting `{n}` placeholders.
///
/// A call-shaped template (`is_call_shaped_template`, e.g. `MOD({0}, {1})`)
/// substitutes each argument's own printed text verbatim — a function call's
/// comma-separated arguments are already unambiguously delimited, so no
/// argument ever needs wrapping. A non-call template (e.g. `{0} - {1}`)
/// parenthesises a compound argument at the substitution site
/// (`is_compound_argument`) and wraps its own output in parentheses so the
/// result composes safely wherever the original call stood.
///
/// Returns `true` when the node was fully printed; `false` only if `args` or
/// `template` don't match what registry validation already guaranteed (never
/// reachable in production — `validate_template` runs at registry
/// construction — but the safe fallback for a hand-built call in a test).
pub fn print_template(
    node: &SyntaxNode,
    template: &str,
    args: &[SyntaxNode],
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let call_shaped = is_call_shaped_template(template);
    let mut body = String::new();
    let mut rest = template;
    loop {
        match rest.find('{') {
            None => {
                body.push_str(rest);
                break;
            }
            Some(pos) => {
                body.push_str(&rest[..pos]);
                let after = &rest[pos + 1..];
                let Some(end) = after.find('}') else {
                    return false;
                };
                let Ok(idx) = after[..end].parse::<usize>() else {
                    return false;
                };
                let Some(arg) = args.get(idx) else {
                    return false;
                };
                let mut arg_sql = String::new();
                print_node(arg, ctx, &mut arg_sql);
                let arg_sql = arg_sql.trim();
                if arg_sql.is_empty() {
                    return false;
                }
                if !call_shaped && is_compound_argument(arg) {
                    body.push('(');
                    body.push_str(arg_sql);
                    body.push(')');
                } else {
                    body.push_str(arg_sql);
                }
                rest = &after[end + 1..];
            }
        }
    }
    if call_shaped {
        out.push_str(&body);
    } else {
        out.push('(');
        out.push_str(&body);
        out.push(')');
    }
    push_trailing_trivia(node, out);
    true
}

/// Lower an ordered-set aggregate's `WITHIN GROUP` spelling to GoogleSQL's
/// two-argument analytic spelling, in place: `PERCENTILE_CONT(0.5) WITHIN
/// GROUP (ORDER BY x)` → `PERCENTILE_CONT(x, 0.5)`. The call's own `OVER`
/// clause is a sibling of `node`, not a child of it, and is left untouched —
/// printed normally by the caller right after this call's replacement text,
/// exactly like `print_bigquery_median`'s windowed branch.
///
/// Returns `true` when the call was printed here, `false` to fall through to
/// the normal path (leaving the call verbatim so the engine rejects it
/// loudly rather than smelt guessing) — unreachable in production, since the
/// compile path refuses a call `within_group_sort_key` cannot read before
/// the printer is ever invoked (`emission_check`), but still the correct
/// fallback for a printer unit test that bypasses that check.
///
/// A `DESC` sort key inverts the fraction (`docs/specs/multi_backend.md`
/// §"Statement-level lowering"); a `NULLS FIRST`/`LAST` modifier the
/// analytic form cannot express is refused upstream rather than reaching
/// this function at all.
fn print_within_group_to_analytic(
    node: &SyntaxNode,
    fc: &FunctionCall,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let Some(name) = fc.name() else {
        return false;
    };
    let canonical = BuiltinRegistry::canonical_name(&name)
        .unwrap_or(name.as_str())
        .to_string();

    let fraction = fc
        .arguments()
        .first()
        .map(|e| print_trimmed(e.syntax(), ctx))
        .unwrap_or_default();
    if fraction.is_empty() {
        return false;
    }

    let Ok((sort_expr, fraction_complement)) = crate::restructure::within_group_sort_key(node)
    else {
        return false;
    };
    let fraction = if fraction_complement {
        format!("(1 - {fraction})")
    } else {
        fraction
    };
    let sort_sql = print_trimmed(&sort_expr, ctx);

    out.push_str(&format!("{canonical}({sort_sql}, {fraction})"));
    push_trailing_trivia(node, out);
    true
}

/// Re-emit the trivia tokens trailing a node whose text a rewrite replaced,
/// so a following sibling (an `OVER` clause, the next select item) does not
/// end up glued to the rewritten text.
fn push_trailing_trivia(node: &SyntaxNode, out: &mut String) {
    // `take_while` must run over the raw `children_with_tokens()` sequence,
    // stopping at the first non-trivia element (token *or* node) — not over
    // a tokens-only view. Filtering nodes out before reversing would erase
    // the node boundary that ought to stop the walk, silently splicing
    // together two separate trivia gaps either side of an intervening child
    // node (e.g. the gap before `WITHIN GROUP (...)` and the gap after it)
    // into one run of "trailing" trivia neither of which the caller meant.
    // Trailing trivia is not necessarily a direct child of `node` — it can be
    // nested inside the last non-trivia child (e.g. an operand expression
    // absorbs the whitespace that follows it as its own trailing token), so
    // this walks the full descendant token stream in document order rather
    // than `node`'s direct children. That also makes it safe when a node has
    // more than one structural child with a trivia gap either side of an
    // intervening child node (e.g. a call's `ARG_LIST` and its `WITHIN GROUP`
    // clause): the reversed walk still stops at the first non-trivia token —
    // here, that clause's own closing `)` — rather than splicing the two
    // separate gaps either side of it into one run.
    let trailing: Vec<_> = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take_while(|t| t.kind().is_trivia())
        .collect();
    for token in trailing.into_iter().rev() {
        out.push_str(token.text());
    }
}

/// Resolve the call's registry entry and dispatch on its emission verdict for
/// this dialect. Returns `true` when the node was fully printed.
///
/// `BuiltinRegistry::resolve` folds case; `FunctionCall::name()` returns the raw
/// source spelling, preserving whatever casing the author used.
pub(crate) fn emit_registered_function(
    node: &SyntaxNode,
    fc: &FunctionCall,
    name: &str,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let Some(sig) = BuiltinRegistry::resolve(name) else {
        return false;
    };
    // Position is decided once, from the source CST, and handed to the
    // registry — the printer never re-derives it (`docs/specs/multi_backend.md`
    // §"Emission is scoped to call position"). A call embedded in a
    // synthesised restructure CTE carries an override, pushed by the
    // restructure printing path (`with_position_override`): its position in
    // the *printed* SQL differs from what `position::classify` would read off
    // its stale original tree location, because that location's `.clone()`
    // still resolves to the original parent (see `ACTIVE_POSITION_OVERRIDES`).
    // Every other call falls through to the ordinary classify-from-source-CST
    // path, unchanged. The topmost ancestor of `node` is the root the
    // classifier resolves named `WINDOW` clauses against.
    let position = active_position_override_for(node).unwrap_or_else(|| {
        let root = node.ancestors().last().unwrap_or_else(|| node.clone());
        classify_position(node, &root)
    });
    match settled_verdict_for(node, sig, position, ctx) {
        // An `Unsupported` entry still prints verbatim; the compile path refuses
        // the model before reaching the printer (see `emission_check`), so a
        // verbatim print here is unreachable in production and harmless in a
        // printer unit test.
        // A statement-level restructure is not an expression-level
        // substitution: it rewrites the enclosing query block's `FROM`
        // around a synthesised CTE, planned before printing ever starts
        // (`docs/specs/multi_backend.md` §"Statement-level lowering"). This
        // per-call print path has nothing to substitute in place, so it
        // prints the call verbatim, exactly like `Unsupported` — the compile
        // path either applies the restructure plan upstream or refuses the
        // model before reaching the printer.
        SettledEmission::Native
        | SettledEmission::Unsupported { .. }
        | SettledEmission::Restructure(_) => false,
        SettledEmission::Rename(new_name) => {
            // The author already wrote the target spelling — via an alias, or
            // in different case. Rewriting it would churn the user's own text
            // (and break DuckDB byte-identity, `architecture.md` §"Print-level
            // identity for the DuckDB dialect": input already using
            // DuckDB-flavoured spellings round-trips byte-identically).
            if name.eq_ignore_ascii_case(new_name) {
                return false;
            }
            print_function_with_renamed(node, ctx, out, new_name);
            true
        }
        SettledEmission::Rewrite(id) => apply_rewrite(id, node, Some(fc), position, ctx, out),
        SettledEmission::Template(template) => {
            let args: Vec<SyntaxNode> = fc
                .arguments()
                .into_iter()
                .map(|e| e.syntax().clone())
                .collect();
            print_template(node, template, &args, ctx, out)
        }
    }
}

pub(crate) fn emit_registered_operator(
    node: &SyntaxNode,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let Some(bin) = BinaryExpr::cast(node.clone()) else {
        return false;
    };
    let Some(op) = bin.operator() else {
        return false;
    };
    let Some(sig) = BuiltinRegistry::resolve(&op) else {
        return false;
    };
    // Operators are never a call in window/aggregate position; their verdicts
    // are stated with `Position::Any`, so there is no position to classify.
    match settled_verdict_for(node, sig, Position::Any, ctx) {
        SettledEmission::Rewrite(id) => apply_rewrite(id, node, None, Position::Any, ctx, out),
        SettledEmission::Template(template) => {
            let (Some(left), Some(right)) = (bin.left(), bin.right()) else {
                return false;
            };
            let args = [left.syntax().clone(), right.syntax().clone()];
            print_template(node, template, &args, ctx, out)
        }
        _ => false,
    }
}

/// The one place a `RewriteId` becomes code. Adding a variant is a compile error
/// here until it is implemented.
fn apply_rewrite(
    id: RewriteId,
    node: &SyntaxNode,
    fc: Option<&FunctionCall>,
    position: Position,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    match id {
        RewriteId::BigQueryMedian => {
            fc.is_some_and(|fc| print_bigquery_median(node, fc, position, ctx, out))
        }
        RewriteId::WithinGroupToAnalytic => {
            fc.is_some_and(|fc| print_within_group_to_analytic(node, fc, ctx, out))
        }
    }
}

fn print_function_with_renamed(
    node: &SyntaxNode,
    ctx: &PrintContext,
    out: &mut String,
    new_name: &str,
) {
    // For a simple function call, the first IDENT token is the name.
    // For a namespaced call (smelt.ref), we'd have already handled it above,
    // so here we only deal with simple IDENT calls.
    let mut replaced = false;
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Token(token) => {
                if !replaced && token.kind() == SyntaxKind::IDENT {
                    out.push_str(new_name);
                    replaced = true;
                } else {
                    out.push_str(token.text());
                }
            }
            SyntaxElement::Node(child_node) => {
                print_node(&child_node, ctx, out);
            }
        }
    }
}
