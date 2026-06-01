//! Compile-time meta-language evaluation on the build path.
//!
//! The analyzer (`smelt-db`) type-checks and validates the meta-language
//! surface (`List<T>`, spread, HOFs, reflection, …) but does not itself
//! lower it to Data-World SQL. This module performs the *expansion* the
//! analyzer's validation presupposes, so that a meta construct the analyzer
//! accepts compiles to plain SQL rather than reaching the database engine
//! verbatim. It is the build-path half of the diagnostic-parity guarantee
//! (`docs/specs/architecture.md` §"Diagnostic parity rule") and realises
//! `meta_language.md` §Semantics "Lists and spread" rule 10 ("the Data-World
//! CST handed to codegen contains no `ARRAY_LITERAL` and no spread node").
//!
//! Currently implemented: in-model **list-spread** expansion in SELECT lists
//! (`SELECT id, ...[a, b] FROM t` → `SELECT id, a, b FROM t`). Higher-order
//! functions, reflection, and config-driven constructs are evaluated by the
//! analyzer but not yet lowered here — see `meta_language.md`
//! §"Known Divergences".

use smelt_parser::ast::ListSpread;
use smelt_parser::SyntaxKind::{LIST_SPREAD, SELECT_ITEM, SELECT_LIST};

/// Run every in-model meta-language build-path expansion over `sql`, returning
/// the rewritten SQL. The single entry point compile sites call before parsing
/// a user model so codegen never sees a meta construct.
///
/// Pure and idempotent: a second pass over already-expanded SQL is a no-op
/// (there are no spread nodes left to rewrite), so it is safe to call at every
/// compile entry point even when SQL flows through several of them.
pub fn expand_in_model_meta(sql: &str) -> String {
    // Cheap guard: the spread token is the only construct this pass rewrites,
    // and it always contains `...`. Skip the reparse for the common case of a
    // model with no spreads at all.
    if !sql.contains("...") {
        return sql.to_string();
    }
    expand_select_list_spreads(sql)
}

/// Expand inline list-literal spreads in SELECT lists to plain comma-separated
/// SELECT items at compile time.
///
/// For every `SELECT_LIST` that contains a `LIST_SPREAD`, the list is rebuilt
/// from its entries: a regular item is kept verbatim, and a spread of an inline
/// list literal `...[e_1, …, e_n]` contributes its `n` element expressions in
/// order. An empty-list spread `...[]` contributes nothing — eliding itself and
/// its adjacent commas (`meta_language.md` §Semantics rule 7).
///
/// A spread whose operand is not an inline list literal (e.g. a `List<T>`
/// variable produced by a HOF) is left untouched: that lowering is not yet
/// implemented, so the whole containing SELECT list is passed through verbatim
/// rather than partially rewritten. The same conservative pass-through applies
/// to a list literal carrying a nested spread, and to any SELECT list whose
/// children are not exclusively items and spreads.
pub fn expand_select_list_spreads(sql: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();

    // Collect (start, end, replacement) edits for every rewritable SELECT_LIST.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for node in root.descendants() {
        if node.kind() != SELECT_LIST {
            continue;
        }
        // Only rewrite lists that actually contain a spread.
        if !node.children().any(|c| c.kind() == LIST_SPREAD) {
            continue;
        }
        // Conservative bail: only rewrite a list whose every child node is a
        // SELECT_ITEM or LIST_SPREAD. Anything else (a bare `*`, an unforeseen
        // node) means our entry-based rebuild could silently drop a child, so
        // we leave the list verbatim.
        if node
            .children()
            .any(|c| c.kind() != SELECT_ITEM && c.kind() != LIST_SPREAD)
        {
            continue;
        }

        let mut items: Vec<String> = Vec::new();
        let mut rewritable = true;
        for child in node.children() {
            if child.kind() == SELECT_ITEM {
                items.push(child.text().to_string().trim().to_string());
                continue;
            }
            // Must be a LIST_SPREAD (the bail above guarantees it).
            let Some(spread) = ListSpread::cast(child.clone()) else {
                rewritable = false;
                break;
            };
            match spread.operand().and_then(|op| op.as_array_literal()) {
                Some(arr) if !has_nested_spread(&child) => {
                    for el in arr.elements() {
                        items.push(el.syntax().text().to_string().trim().to_string());
                    }
                }
                // Non-literal operand, or a literal carrying a nested spread:
                // not yet lowered here. Leave the whole list verbatim.
                _ => {
                    rewritable = false;
                    break;
                }
            }
        }
        if !rewritable {
            continue;
        }
        // Degenerate case: every entry was an empty spread, so the list would
        // become empty (`SELECT ...[] FROM t` → `SELECT  FROM t`, which is not
        // valid SQL). Leave it verbatim rather than emit broken SQL — a SELECT
        // with no projected columns is unbuildable either way.
        if items.is_empty() {
            continue;
        }

        // Preserve the node's surrounding whitespace exactly — the SELECT_LIST
        // range can include trailing trivia (the newline before FROM) — and
        // replace only the content with the joined items.
        let original = node.text().to_string();
        let lead = &original[..original.len() - original.trim_start().len()];
        let trail = &original[original.trim_end().len()..];
        let replacement = format!("{lead}{}{trail}", items.join(", "));
        let range = node.text_range();
        edits.push((range.start().into(), range.end().into(), replacement));
    }

    if edits.is_empty() {
        return sql.to_string();
    }

    // Apply edits from the highest offset down so earlier offsets stay valid.
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = sql.to_string();
    for (start, end, repl) in edits {
        out.replace_range(start..end, &repl);
    }
    out
}

/// True if the `LIST_SPREAD` node's operand list literal contains a nested
/// spread element (`...[a, ...xs, b]`) — a shape this pass does not yet lower.
/// The spread node's own `LIST_SPREAD` is the first in its subtree; any further
/// `LIST_SPREAD` descendant is a nested spread inside the operand literal.
fn has_nested_spread(spread_node: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    spread_node
        .descendants()
        .filter(|n| n.kind() == LIST_SPREAD)
        .count()
        > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_spread_expands_to_items() {
        let sql = "SELECT\n    id,\n    ...[1, 2, 3]\nFROM smelt.sources.raw.users\n";
        let out = expand_select_list_spreads(sql);
        assert_eq!(
            out,
            "SELECT\n    id, 1, 2, 3\nFROM smelt.sources.raw.users\n"
        );
    }

    #[test]
    fn column_ref_spread_expands_to_items() {
        let sql = "SELECT\n    id,\n    ...[name, email]\nFROM t\n";
        let out = expand_select_list_spreads(sql);
        assert_eq!(out, "SELECT\n    id, name, email\nFROM t\n");
    }

    #[test]
    fn multi_spread_with_surrounding_items() {
        let sql = "SELECT\n    id,\n    ...[name, email],\n    created_at\nFROM t\n";
        let out = expand_select_list_spreads(sql);
        assert_eq!(out, "SELECT\n    id, name, email, created_at\nFROM t\n");
    }

    #[test]
    fn empty_spread_elides_with_adjacent_commas() {
        let sql = "SELECT\n    id,\n    ...[],\n    created_at\nFROM t\n";
        let out = expand_select_list_spreads(sql);
        assert_eq!(out, "SELECT\n    id, created_at\nFROM t\n");
    }

    #[test]
    fn no_spread_is_unchanged() {
        let sql = "SELECT id, name FROM t\n";
        assert_eq!(expand_select_list_spreads(sql), sql);
        assert_eq!(expand_in_model_meta(sql), sql);
    }

    #[test]
    fn idempotent_second_pass_is_noop() {
        let sql = "SELECT id, ...[a, b] FROM t";
        let once = expand_select_list_spreads(sql);
        assert_eq!(once, "SELECT id, a, b FROM t");
        assert_eq!(expand_select_list_spreads(&once), once);
    }

    #[test]
    fn all_empty_spread_list_left_verbatim() {
        // A SELECT whose only items are empty spreads would rebuild to an
        // empty list (`SELECT  FROM t`); leave it verbatim instead.
        let sql = "SELECT ...[] FROM t";
        assert_eq!(expand_select_list_spreads(sql), sql);
    }

    #[test]
    fn non_literal_spread_operand_left_verbatim() {
        // A `List<T>` variable spread is not yet lowered here; the list is
        // passed through unchanged rather than partially rewritten.
        let sql = "SELECT id, ...xs FROM t";
        assert_eq!(expand_select_list_spreads(sql), sql);
    }

    #[test]
    fn guard_skips_reparse_without_spread_token() {
        // expand_in_model_meta short-circuits when there is no `...`.
        let sql = "SELECT a, b, c FROM t";
        assert_eq!(expand_in_model_meta(sql), sql);
    }

    #[test]
    fn multiple_select_lists_each_expanded() {
        let sql = "SELECT ...[a, b] FROM (SELECT ...[c, d] FROM t) s";
        let out = expand_select_list_spreads(sql);
        assert_eq!(out, "SELECT a, b FROM (SELECT c, d FROM t) s");
    }
}
