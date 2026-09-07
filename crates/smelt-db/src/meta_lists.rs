//! Pure classifiers for meta-language list values appearing in scalar
//! positions (`meta_language.md` §Semantics "Lists and spread"). No Salsa
//! access; called by `file_check`'s diagnostic orchestrator.

/// True if a top-level SELECT-item expression evaluates to a bare, unconsumed
/// `List<T>` — used to emit `MetaListInScalarPosition` (`meta_language.md`
/// §Semantics "Lists and spread" rule 10). List-yielding shapes:
///   - a bare list literal (`[1, 2, 3]`);
///   - a top-level `map` / `filter` HOF call (each produces a `List<U>`);
///   - a pipe whose outermost call is `map` / `filter` (`xs |> map(…)`).
///
/// `reduce` collapses a list to a scalar, so a `reduce(...)` item is consumed
/// and not list-yielding. A spread (`...xs`) is a `LIST_SPREAD` node, not a
/// select-item expression, so it never reaches this check.
///
/// A `smelt.config.load_yaml` / `load_json` call whose schema is `List<…>` or
/// `Map<Text, …>` yields a collection value (`List<record>` / `Map<Text,
/// record>`); left bare in a select item it is likewise unconsumed
/// (`meta_config_loading.md` — the loader is governed by the same
/// lists-must-be-consumed rule). A record-schema loader returns a single record,
/// not a collection, and is not flagged here.
pub(crate) fn select_item_yields_bare_list(expr: &smelt_parser::ast::Expr) -> bool {
    // Case 1: a bare list literal directly in the select item.
    if expr.as_array_literal().is_some() {
        return true;
    }
    // Case 1b: a bare collection-valued loader call (`load_yaml` / `load_json`
    // with a `List<…>` / `Map<…>` schema argument).
    if loader_call_yields_collection(expr) {
        return true;
    }
    // Case 2a: a top-level `map` / `filter` HOF call.
    if let Some(call) = expr.as_function_call() {
        if hof_call_is_list_yielding(&call) {
            return true;
        }
    }
    // Case 2b: a pipe whose outermost RHS call is `map` / `filter`.
    let node = expr.syntax();
    let pipe = node
        .children()
        .find_map(smelt_parser::ast::PipeExpr::cast)
        .or_else(|| smelt_parser::ast::PipeExpr::cast(node.clone()));
    if let Some(pipe) = pipe {
        if let Some(rhs) = pipe.rhs() {
            if let Some(call) = rhs.as_function_call() {
                if hof_call_is_list_yielding(&call) {
                    return true;
                }
            }
        }
    }
    false
}

/// True if `expr` is a `smelt.config.load_yaml` / `load_json` call whose schema
/// argument (the second positional argument) is a `List<…>` or `Map<…>` type —
/// i.e. the loader's value is a collection that must be consumed before it
/// reaches a Data-World scalar position. A record-schema loader (`{…}` or a
/// named record) returns a single record and is excluded.
fn loader_call_yields_collection(expr: &smelt_parser::ast::Expr) -> bool {
    let Some(call) = expr.as_smelt_path_call() else {
        return false;
    };
    let segs = call.segments();
    if segs.len() != 2 || segs[0].to_lowercase() != "config" {
        return false;
    }
    let loader = segs[1].to_lowercase();
    if loader != "load_yaml" && loader != "load_json" {
        return false;
    }
    let Some(schema_arg) = call
        .arg_list()
        .and_then(|a| a.positional_args().into_iter().nth(1))
    else {
        return false;
    };
    let schema_text = schema_arg.syntax().text().to_string();
    let trimmed = schema_text.trim();
    trimmed.starts_with("List<") || trimmed.starts_with("Map<")
}

/// True if a function call is a `map` or `filter` HOF (the list-yielding HOFs).
/// `filter` lexes as a keyword (`FILTER_KW`), so `name()` may be `None`; fall
/// back to the call's first token text (mirrors `hof.rs`).
fn hof_call_is_list_yielding(call: &smelt_parser::ast::FunctionCall) -> bool {
    let name = call.name().map(|n| n.to_lowercase()).or_else(|| {
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .map(|t| t.text().to_lowercase())
            .find(|t| t == "map" || t == "filter")
    });
    matches!(name.as_deref(), Some("map") | Some("filter"))
}
