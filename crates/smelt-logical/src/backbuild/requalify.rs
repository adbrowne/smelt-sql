//! CST-fragment rewrite: requalify a derivable expression's column
//! references to their stored 1:1 representative names, producing the SQL
//! text to splice into a self-read `UPDATE ... SET c = <this>` assignment
//! (research `docs/research/20260802-backbuild-synthesis.md` §3 "Alias
//! requalification is a CST rewrite, not string surgery" and §4 intro
//! "Derivability representatives — one uniform rule").
//!
//! This is a positional rewrite keyed on each column reference's own
//! [`TextRange`] (found via the typed AST — the same recognised shapes
//! `analysis::model_diff::collect_dependencies` walks) — not a string
//! search-and-replace: a representative name that happens to appear inside
//! a string literal, comment, or unrelated identifier is never touched.
//! Every byte outside a replaced span — operators, function names,
//! literals, whitespace/comments — is copied through unchanged from the
//! original source text, in the spirit of the exclusion-range text-rebuild
//! `analysis::walk::own_region_text_excluding_self_relations` already uses
//! elsewhere in this crate (replacement here rather than exclusion).

use std::collections::BTreeSet;

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr, SyntaxKind, TextRange};

/// Requalify every column reference inside `expr` against
/// `representatives` (the exact set of stored 1:1 representative output
/// column names — bare pull-throughs unchanged between both definitions,
/// per the uniform rule).
///
/// Walks the same recognised expression shapes as
/// [`crate::analysis::model_diff::collect_dependencies`] (column reference,
/// function call, binary, `CASE`, `CAST`, literal). By the time this is
/// called, that walk has already proven the whole expression is built from
/// exactly these shapes and that every dependency resolves to a name in
/// `representatives` — so an unrecognised shape or an unresolved name here
/// would be an internal-invariant violation, not a legitimate refusal path.
/// It is still reported as an `Err` rather than panicking (fail-loud
/// discipline: `docs/specs/architecture.md` §"Fail-loud discipline").
///
/// A qualified reference (`o.price`) is rewritten to its bare
/// representative name (`price`); an already-bare reference matching a
/// representative is left as-is (rewritten to itself, byte-identical).
pub fn requalify(expr: &Expr, representatives: &BTreeSet<String>) -> Result<String, String> {
    collect_and_splice(expr, &|col| {
        representatives.get(col.name()).cloned().ok_or_else(|| {
            format!(
                "column '{}' has no stored representative to requalify against",
                col.name()
            )
        })
    })
}

/// Requalify every column reference inside `expr` to the upstream statement
/// alias used inside a backbuild `UPDATE ... FROM <upstream> u` (research
/// `docs/research/20260802-backbuild-synthesis.md` §4 B3/D2 and §3 "Alias
/// requalification is a CST rewrite, not string surgery" — the "statement-
/// context aliasing (`t.` / `u.`)" extension). Every reference must be
/// qualified with exactly `source_alias` — the single FROM-tree alias the
/// B3/D2 grain-link proof (`classify.rs`'s `admit_upstream_pullthrough`)
/// already bound this expression's dependencies to — and is rewritten to
/// `<upstream_alias>.<raw column name>` (e.g. `o.discount` with
/// `source_alias = "o"`, `upstream_alias = "u"` becomes `u.discount`).
///
/// A reference under a different qualifier, or with no qualifier at all,
/// would mean the caller's single-alias admission proof was unsound — an
/// internal-invariant violation, reported as `Err` (fail-loud discipline)
/// rather than silently reproduced or panicking.
pub fn requalify_upstream(
    expr: &Expr,
    source_alias: &str,
    upstream_alias: &str,
) -> Result<String, String> {
    collect_and_splice(expr, &|col| match col.qualifier() {
        Some(q) if q == source_alias => Ok(format!("{upstream_alias}.{}", col.name())),
        _ => Err(format!(
            "column '{}' is not qualified with the upstream alias '{source_alias}' the \
             grain-link proof bound this expression to",
            col.name()
        )),
    })
}

fn collect_and_splice(
    expr: &Expr,
    resolve: &impl Fn(&ColumnRef) -> Result<String, String>,
) -> Result<String, String> {
    let mut spans: Vec<(TextRange, String)> = Vec::new();
    collect_replacements(expr, resolve, &mut spans)?;
    spans.sort_by_key(|(range, _)| range.start());
    Ok(splice(expr.syntax(), &spans))
}

/// Mirrors `analysis::model_diff::walk`'s recognised shapes exactly, but
/// records `(range, replacement)` pairs for column-reference leaves —
/// resolved by the caller-supplied `resolve` closure — instead of
/// collecting dependency names. Shared by [`requalify`] (self-read: replace
/// with a bare representative name) and [`requalify_upstream`] (upstream
/// statement-context: replace with `<alias>.<name>`); only the leaf
/// resolution differs, the traversal is identical.
fn collect_replacements(
    expr: &Expr,
    resolve: &impl Fn(&ColumnRef) -> Result<String, String>,
    out: &mut Vec<(TextRange, String)>,
) -> Result<(), String> {
    if let Some(col) = expr.as_column_ref() {
        let replacement = resolve(&col)?;
        out.push((trimmed_range(expr.syntax()), replacement));
        return Ok(());
    }

    if let Some(func) = expr.as_function_call() {
        for arg in func.arguments() {
            collect_replacements(&arg, resolve, out)?;
        }
        return Ok(());
    }

    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            collect_replacements(&side, resolve, out)?;
        }
        return Ok(());
    }

    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            collect_replacements(&value, resolve, out)?;
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                collect_replacements(&arm, resolve, out)?;
            }
        }
        if let Some(else_expr) = case.else_expr() {
            collect_replacements(&else_expr, resolve, out)?;
        }
        return Ok(());
    }

    if let Some(cast) = expr.as_cast() {
        return match cast.expression() {
            Some(inner) => collect_replacements(&inner, resolve, out),
            None => Err("CAST has no inner expression to requalify".to_string()),
        };
    }

    // A leaf with no identifier tokens at all is a literal — nothing to
    // requalify. Anything else is a shape this walk does not recognise —
    // fail closed rather than silently reproduce it verbatim (it could
    // contain an unqualified reference that needed requalifying).
    let has_ident = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::IDENT);
    if has_ident {
        return Err("expression shape is not recognised by the requalification walk".to_string());
    }
    Ok(())
}

/// The range spanning `node`'s own first through last *non-trivia* token —
/// i.e. `node`'s [`TextRange`] with any leading/trailing whitespace or
/// comments the grammar attached to the node itself (rather than to a
/// sibling token) trimmed off. Column-reference leaf nodes can carry
/// trailing trivia this way; using the raw node range as a replacement span
/// would silently eat that trivia (e.g. the space between `price` and `*`
/// in `price * qty`) along with the tokens being replaced.
fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let mut first = None;
    let mut last = None;
    for tok in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
    {
        if first.is_none() {
            first = Some(tok.text_range().start());
        }
        last = Some(tok.text_range().end());
    }
    match (first, last) {
        (Some(start), Some(end)) => TextRange::new(start, end),
        _ => node.text_range(),
    }
}

/// Rebuild `node`'s own (trivia-trimmed) text, splicing in each recorded
/// `(range, replacement)` span — sorted by start, non-overlapping — and
/// copying every byte outside a replaced span through unchanged, including
/// whitespace/comments. A positional byte-range splice over the original
/// source text, not a per-node tree reconstruction: every token this walk
/// does not touch (operators, function names, literals, punctuation,
/// trivia) survives byte-for-byte.
fn splice(node: &SyntaxNode, spans: &[(TextRange, String)]) -> String {
    let node_start = u32::from(node.text_range().start());
    let full_text = node.text().to_string();
    let content = trimmed_range(node);
    let content_start = u32::from(content.start());
    let content_end = u32::from(content.end());

    let mut out = String::new();
    let mut cursor = content_start;
    for (range, replacement) in spans {
        let start = u32::from(range.start());
        let end = u32::from(range.end());
        if start < cursor {
            // Overlapping/out-of-order span — should not happen for the
            // disjoint leaf shapes this walk produces (fail-soft: keep
            // going rather than corrupt earlier output).
            continue;
        }
        out.push_str(&full_text[(cursor - node_start) as usize..(start - node_start) as usize]);
        out.push_str(replacement);
        cursor = end;
    }
    out.push_str(&full_text[(cursor - node_start) as usize..(content_end - node_start) as usize]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(sql_expr: &str) -> Expr {
        let sql = format!("SELECT {sql_expr} AS v FROM t");
        let parse = smelt_parser::parse(&sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let select = file.select_stmt().expect("select");
        select
            .select_list()
            .expect("select list")
            .items()
            .next()
            .expect("item")
            .expression()
            .expect("expression")
    }

    fn reps(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn bare_reference_to_a_representative_is_unchanged() {
        let e = expr("price");
        let out = requalify(&e, &reps(&["price"])).expect("requalify");
        assert_eq!(out, "price");
    }

    #[test]
    fn qualified_reference_is_rewritten_to_the_bare_representative_name() {
        let e = expr("o.price");
        let out = requalify(&e, &reps(&["price"])).expect("requalify");
        assert_eq!(out, "price");
    }

    #[test]
    fn arithmetic_over_qualified_columns_rewrites_only_the_column_references() {
        let e = expr("o.price * o.qty");
        let out = requalify(&e, &reps(&["price", "qty"])).expect("requalify");
        assert_eq!(out, "price * qty");
    }

    #[test]
    fn function_call_arguments_are_requalified_and_the_call_shape_is_preserved() {
        let e = expr("COALESCE(o.price, 0)");
        let out = requalify(&e, &reps(&["price"])).expect("requalify");
        assert_eq!(out, "COALESCE(price, 0)");
    }

    #[test]
    fn case_expression_arms_are_requalified() {
        let e = expr("CASE WHEN o.price > 0 THEN o.price ELSE o.qty END");
        let out = requalify(&e, &reps(&["price", "qty"])).expect("requalify");
        assert_eq!(out, "CASE WHEN price > 0 THEN price ELSE qty END");
    }

    #[test]
    fn cast_inner_expression_is_requalified() {
        let e = expr("CAST(o.price AS DOUBLE)");
        let out = requalify(&e, &reps(&["price"])).expect("requalify");
        assert_eq!(out, "CAST(price AS DOUBLE)");
    }

    #[test]
    fn a_literal_is_reproduced_verbatim() {
        let e = expr("'active'");
        let out = requalify(&e, &BTreeSet::new()).expect("requalify");
        assert_eq!(out, "'active'");
    }

    #[test]
    fn a_string_literal_containing_a_representative_name_is_never_touched() {
        // The whole point of a positional (not string-search) rewrite: a
        // representative name that happens to appear inside a string
        // literal must never be substituted.
        let e = expr("CONCAT('price', o.price)");
        let out = requalify(&e, &reps(&["price"])).expect("requalify");
        assert_eq!(out, "CONCAT('price', price)");
    }

    #[test]
    fn a_dependency_with_no_representative_fails_closed() {
        let e = expr("o.region_name");
        let err = requalify(&e, &BTreeSet::new()).unwrap_err();
        assert!(err.contains("region_name"));
    }

    #[test]
    fn requalify_upstream_rewrites_the_bound_alias_to_the_statement_alias() {
        let e = expr("o.discount");
        let out = requalify_upstream(&e, "o", "u").expect("requalify_upstream");
        assert_eq!(out, "u.discount");
    }

    #[test]
    fn requalify_upstream_rewrites_every_reference_under_the_bound_alias() {
        let e = expr("o.price * o.qty");
        let out = requalify_upstream(&e, "o", "u").expect("requalify_upstream");
        assert_eq!(out, "u.price * u.qty");
    }

    #[test]
    fn requalify_upstream_refuses_a_reference_under_a_different_alias() {
        let e = expr("o2.discount");
        let err = requalify_upstream(&e, "o1", "u").unwrap_err();
        assert!(err.contains("discount"));
    }

    #[test]
    fn requalify_upstream_refuses_an_unqualified_reference() {
        let e = expr("discount");
        let err = requalify_upstream(&e, "o", "u").unwrap_err();
        assert!(err.contains("discount"));
    }
}
