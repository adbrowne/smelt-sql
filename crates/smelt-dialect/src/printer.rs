//! Dialect-aware CST printer.
//!
//! Walks a Rowan CST and emits SQL text, performing dialect-specific rewrites:
//! - `smelt.<path>` → resolved table name (via `SmeltPathRef`/`SmeltPathCall`)
//! - QUALIFY → subquery rewrite (when `!caps.supports_qualify`)
//! - ARRAY[1,2,3] → ARRAY(1,2,3) (when `!caps.supports_array_literal`)
//! - DATE '2024-01-01' → DATE('2024-01-01') (when `!caps.supports_date_literal`)
//! - expr::type → CAST(expr AS type) (when `!caps.supports_double_colon_cast`)
//!
//! The default behavior is verbatim: tokens (including whitespace and comments)
//! are emitted exactly as they appear in the source. This guarantees an identity
//! property for DuckDB with no refs/sources.

use smelt_parser::ast::{SmeltAsStructCall, SmeltFnCall, SmeltPathCall, SmeltPathRef};
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::{CastExpr, FunctionCall};

use std::collections::{HashMap, HashSet};

use crate::{BackendCapabilities, SqlDialect};

/// Emitter closure type for `smelt.as_struct(alias [EXCEPT cols])`.
/// Called with `(alias, except_columns)` → emitted SQL, or `None` to pass through.
pub type AsStructEmitter<'a> = Box<dyn Fn(&str, &[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.fn.*` calls.
/// Called with `(fn_name, positional_arg_sqls, named_args)` → expanded SQL, or `None`.
pub type SmeltFnExpander<'a> =
    Box<dyn Fn(&str, Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Resolver closure type for `smelt.<path>` value references (`SMELT_PATH_REF`).
/// Called with path segments (everything after `smelt`); returns backend SQL or `None` to
/// emit verbatim.
pub type SmeltPathRefResolver<'a> = Box<dyn Fn(&[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
/// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` → expanded SQL, or `None`
/// to emit verbatim.
pub type SmeltPathCallExpander<'a> =
    Box<dyn Fn(&[String], Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Context for dialect-aware printing.
pub struct PrintContext<'a> {
    pub dialect: &'a SqlDialect,
    pub capabilities: &'a BackendCapabilities,
    pub schema: &'a str,
    /// Model names that are ephemeral — refs to these emit `__smelt_{name}` instead of `schema.name`.
    pub ephemeral_models: HashSet<&'a str>,
    /// Cross-engine refs: model_name -> `read_parquet('{path}/**/*.parquet', hive_partitioning=true)`.
    /// When a ref is in this map, the parquet expression is emitted instead of `schema.model`.
    pub cross_engine_refs: HashMap<String, String>,
    /// Emitter for `smelt.as_struct(alias [EXCEPT cols])`.
    ///
    /// Called with `(alias, except_columns)` and returns the backend-specific SQL string.
    /// `None` = pass through verbatim (backward compat for tests / contexts without schema info).
    pub smelt_as_struct: Option<AsStructEmitter<'a>>,
    /// Expander for `smelt.fn.*` calls.
    ///
    /// Called with `(fn_name, positional_arg_sqls, named_args)` and returns the expanded SQL.
    /// `None` = pass through verbatim (backward compat for tests / contexts without function info).
    pub smelt_fn: Option<SmeltFnExpander<'a>>,
    /// Resolver for `smelt.<path>` value references (`SMELT_PATH_REF`).
    ///
    /// Called with path segments (everything after `smelt`) and returns the backend SQL string.
    /// `None` = pass through verbatim (backward compat for callers that have not configured
    /// path resolution).
    pub smelt_path_ref: Option<SmeltPathRefResolver<'a>>,
    /// Expander for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
    ///
    /// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` and returns the
    /// expanded SQL. `None` = pass through verbatim.
    pub smelt_path_call: Option<SmeltPathCallExpander<'a>>,
}

/// Print a CST node as dialect-specific SQL.
pub fn print(node: &SyntaxNode, ctx: &PrintContext) -> String {
    let mut out = String::new();
    print_node(node, ctx, &mut out);
    out
}

fn print_node(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    match node.kind() {
        SyntaxKind::SMELT_AS_STRUCT_CALL => {
            if let Some(ref emitter) = ctx.smelt_as_struct {
                if let Some(call) = SmeltAsStructCall::cast(node.clone()) {
                    if let Some(alias) = call.alias() {
                        let except = call.except_columns();
                        if let Some(sql) = emitter(&alias, &except) {
                            out.push_str(&sql);
                            return;
                        }
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_FN_CALL => {
            if let Some(ref expander) = ctx.smelt_fn {
                if let Some(call) = SmeltFnCall::cast(node.clone()) {
                    // Extract the leaf function name (last segment after smelt.fn.)
                    let segments = call.call_path().map(|p| p.segments()).unwrap_or_default();
                    if let Some(fn_name) = segments.last().cloned() {
                        // Extract positional arg SQL strings by printing each arg through ctx
                        let positional_sqls: Vec<String> = call
                            .arg_list()
                            .map(|al| {
                                al.positional_args()
                                    .into_iter()
                                    .map(|arg| {
                                        let mut s = String::new();
                                        print_node(arg.syntax(), ctx, &mut s);
                                        s
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // Extract named args as (name, sql) pairs
                        let named_sqls: Vec<(String, String)> = call
                            .arg_list()
                            .map(|al| {
                                al.named_params()
                                    .filter_map(|np| {
                                        let name = np.name()?;
                                        let expr = np.value_expr()?;
                                        let mut s = String::new();
                                        print_node(expr.syntax(), ctx, &mut s);
                                        Some((name, s))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let Some(expanded) = expander(&fn_name, positional_sqls, named_sqls) {
                            // Re-parse the expanded SQL so smelt.ref() etc. in the body get rewritten.
                            let reparsed = smelt_parser::parse(&expanded);
                            print_node(&reparsed.syntax(), ctx, out);
                            return;
                        }
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_REF => {
            if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
                let segs = path_ref.segments();

                // Try explicit resolver first.
                let resolved = ctx
                    .smelt_path_ref
                    .as_ref()
                    .and_then(|resolver| resolver(&segs));

                // Fall back to built-in resolution based on the path namespace:
                //   smelt.models.<name>   → schema.name  (or cross-engine / ephemeral)
                //   smelt.sources.<src>.<tbl>  → src.tbl
                let resolved = resolved.or_else(|| match segs.as_slice() {
                    [ns, name] if ns == "models" => {
                        if ctx.ephemeral_models.contains(name.as_str()) {
                            Some(format!("__smelt_{}", name))
                        } else if let Some(parquet_expr) = ctx.cross_engine_refs.get(name.as_str())
                        {
                            Some(parquet_expr.clone())
                        } else {
                            Some(format!("{}.{}", ctx.schema, name))
                        }
                    }
                    [ns, src, tbl] if ns == "sources" => Some(format!("{}.{}", src, tbl)),
                    _ => None,
                });

                if let Some(sql) = resolved {
                    out.push_str(&sql);
                    // Re-emit trailing trivia (whitespace/comments) captured
                    // inside this node by the parser's look-ahead skip_trivia
                    // call. These are direct-child tokens after the SMELT_PATH
                    // sub-node (e.g. the space before an `AS` alias).
                    for child in node.children_with_tokens() {
                        if let SyntaxElement::Token(t) = child {
                            if t.kind().is_trivia() {
                                out.push_str(t.text());
                            }
                        }
                    }
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_CALL => {
            if let (Some(ref expander), Some(path_call)) =
                (&ctx.smelt_path_call, SmeltPathCall::cast(node.clone()))
            {
                let segs = path_call.segments();
                let positional: Vec<String> = path_call
                    .arg_list()
                    .map(|al| {
                        al.positional_args()
                            .into_iter()
                            .map(|arg| {
                                let mut s = String::new();
                                print_node(arg.syntax(), ctx, &mut s);
                                s
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let named: Vec<(String, String)> = path_call
                    .arg_list()
                    .map(|al| {
                        al.named_params()
                            .filter_map(|np| {
                                let name = np.name()?;
                                let expr = np.value_expr()?;
                                let mut s = String::new();
                                print_node(expr.syntax(), ctx, &mut s);
                                Some((name, s))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(expanded) = expander(&segs, positional, named) {
                    let reparsed = smelt_parser::parse(&expanded);
                    print_node(&reparsed.syntax(), ctx, out);
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::FUNCTION_CALL => {
            if let Some(fc) = FunctionCall::cast(node.clone()) {
                // Function name remapping per dialect
                if let Some(name) = fc.name() {
                    if let Some(new_name) = remap_function_name(ctx.dialect, &name) {
                        print_function_with_renamed(node, ctx, out, new_name);
                        return;
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SELECT_STMT if !ctx.capabilities.supports_qualify => {
            print_select_with_qualify_rewrite(node, ctx, out);
        }
        SyntaxKind::ARRAY_LITERAL if !ctx.capabilities.supports_array_literal => {
            print_array_rewrite(node, ctx, out);
        }
        SyntaxKind::SELECT_LIST | SyntaxKind::GROUP_BY_CLAUSE
            if !ctx.capabilities.supports_trailing_commas =>
        {
            print_strip_trailing_commas(node, ctx, out);
        }
        SyntaxKind::CAST_EXPR if !ctx.capabilities.supports_double_colon_cast => {
            print_cast_rewrite(node, ctx, out);
        }
        _ => {
            print_children(node, ctx, out);
        }
    }
}

/// Walk children with index-based iteration, allowing look-ahead for DATE literal rewrite.
fn print_children(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                // DATE literal rewrite: DATE 'value' → DATE('value')
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Look ahead in children for optional whitespace followed by a STRING token.
/// Returns (index_of_string, string_text) if found.
fn find_string_after(children: &[SyntaxElement], start: usize) -> Option<(usize, String)> {
    let mut j = start;
    while j < children.len() {
        match &children[j] {
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE => {
                j += 1;
            }
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::STRING => {
                return Some((j, t.text().to_string()));
            }
            _ => return None,
        }
    }
    None
}

/// Rewrite expr::type → CAST(expr AS type) when backend doesn't support ::.
/// If it's already CAST(...) syntax, pass through verbatim.
fn print_cast_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let Some(cast) = CastExpr::cast(node.clone()) else {
        print_children(node, ctx, out);
        return;
    };

    if !cast.is_double_colon_cast() {
        // Already CAST(expr AS type) syntax — pass through
        print_children(node, ctx, out);
        return;
    }

    // Partition children into: expr (before ::), type (TYPE_SPEC node), trailing whitespace.
    // We emit CAST(expr AS type) followed by any trailing whitespace.
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();

    // Find the :: token index
    let dc_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::DOUBLE_COLON));
    let Some(dc_idx) = dc_idx else {
        print_children(node, ctx, out);
        return;
    };

    // Find the TYPE_SPEC node index
    let type_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Node(n) if n.kind() == SyntaxKind::TYPE_SPEC));

    out.push_str("CAST(");

    // Print expression (children before ::)
    for child in &children[..dc_idx] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }

    out.push_str(" AS ");

    // Print TYPE_SPEC, moving any trailing whitespace outside the closing paren
    let mut type_text = String::new();
    if let Some(ti) = type_idx {
        if let SyntaxElement::Node(n) = &children[ti] {
            print_node(n, ctx, &mut type_text);
        }
    }
    let trimmed = type_text.trim_end();
    let trailing = &type_text[trimmed.len()..];
    out.push_str(trimmed);
    out.push(')');
    out.push_str(trailing);

    // Print any remaining children after TYPE_SPEC (unlikely but defensive)
    let after = type_idx.map(|ti| ti + 1).unwrap_or(children.len());
    for child in &children[after..] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }
}

/// Rewrite ARRAY[1,2,3] → ARRAY(1,2,3).
fn print_array_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Token(token) => match token.kind() {
                SyntaxKind::LBRACKET => out.push('('),
                SyntaxKind::RBRACKET => out.push(')'),
                _ => out.push_str(token.text()),
            },
            SyntaxElement::Node(child_node) => {
                print_node(&child_node, ctx, out);
            }
        }
    }
}

/// Handle SELECT with QUALIFY → subquery rewrite when backend doesn't support QUALIFY.
fn print_select_with_qualify_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let has_qualify = node
        .children()
        .any(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE);

    if !has_qualify {
        print_children(node, ctx, out);
        return;
    }

    // Extract the QUALIFY expression
    let qualify_expr = node
        .children()
        .find(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE)
        .and_then(|qc| {
            let mut found_kw = false;
            let mut expr_parts = Vec::new();
            for child in qc.children_with_tokens() {
                match child {
                    SyntaxElement::Token(t) => {
                        if t.kind() == SyntaxKind::QUALIFY_KW {
                            found_kw = true;
                        } else if found_kw {
                            expr_parts.push(t.text().to_string());
                        }
                    }
                    SyntaxElement::Node(n) => {
                        if found_kw {
                            let mut s = String::new();
                            print_node(&n, ctx, &mut s);
                            expr_parts.push(s);
                        }
                    }
                }
            }
            let trimmed = expr_parts.join("").trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    let Some(qualify_expr) = qualify_expr else {
        print_children(node, ctx, out);
        return;
    };

    // Wrap: SELECT * FROM (inner_select_without_qualify) _q WHERE qualify_expr
    out.push_str("SELECT * FROM (");
    print_children_skip_qualify(node, ctx, out);
    out.push_str(") _q WHERE ");
    out.push_str(&qualify_expr);
}

/// Print a SELECT statement's children, skipping the QUALIFY clause.
fn print_children_skip_qualify(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                if child_node.kind() == SyntaxKind::QUALIFY_CLAUSE {
                    i += 1;
                    continue;
                }
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Print children of a SELECT_LIST or GROUP_BY_CLAUSE, stripping trailing commas.
fn print_strip_trailing_commas(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    for (i, child) in children.iter().enumerate() {
        match child {
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMA => {
                // Look ahead: is there any non-whitespace child after this comma?
                let has_more = children[i + 1..].iter().any(
                    |c| !matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE),
                );
                if has_more {
                    out.push_str(token.text());
                }
                // else: trailing comma — skip it (but keep any following whitespace)
            }
            SyntaxElement::Token(token) => {
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
    }
}

/// Print a FUNCTION_CALL node with the function name replaced by `new_name`.
/// Remap a function name for a specific dialect.
/// Returns `Some(new_name)` if the function should be renamed, `None` to keep as-is.
fn remap_function_name<'a>(dialect: &SqlDialect, name: &str) -> Option<&'a str> {
    match dialect {
        SqlDialect::DuckDB => {
            if name.eq_ignore_ascii_case("EXPLODE") {
                Some("UNNEST")
            } else if name.eq_ignore_ascii_case("EVERY") {
                Some("BOOL_AND")
            } else {
                None
            }
        }
        SqlDialect::PostgreSQL => {
            if name.eq_ignore_ascii_case("EXPLODE") {
                Some("UNNEST")
            } else {
                None
            }
        }
        SqlDialect::SparkSQL => {
            if name.eq_ignore_ascii_case("UNNEST") {
                Some("EXPLODE")
            } else if name.eq_ignore_ascii_case("BOOL_AND") {
                Some("EVERY")
            } else if name.eq_ignore_ascii_case("BOOL_OR") {
                Some("SOME")
            } else {
                None
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::parse;

    fn duckdb_ctx() -> (SqlDialect, BackendCapabilities) {
        (SqlDialect::DuckDB, BackendCapabilities::duckdb())
    }

    fn postgresql_ctx() -> (SqlDialect, BackendCapabilities) {
        (SqlDialect::PostgreSQL, BackendCapabilities::postgresql())
    }

    fn spark_ctx() -> (SqlDialect, BackendCapabilities) {
        (SqlDialect::SparkSQL, BackendCapabilities::spark())
    }

    fn print_with(
        sql: &str,
        dialect: &SqlDialect,
        caps: &BackendCapabilities,
        schema: &str,
    ) -> String {
        let parsed = parse(sql);
        let ctx = PrintContext {
            dialect,
            capabilities: caps,
            schema,
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        print(&parsed.syntax(), &ctx)
    }

    // ===== Identity tests =====

    #[test]
    fn test_identity_simple_select() {
        let sql = "SELECT * FROM users";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_complex_query() {
        let sql = "SELECT u.id, COUNT(*) AS cnt\nFROM users u\nWHERE u.active = 1\nGROUP BY u.id\nHAVING COUNT(*) > 5\nORDER BY cnt DESC\nLIMIT 10";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_with_comments() {
        let sql = "-- This is a comment\nSELECT * FROM users";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_with_cte() {
        let sql = "WITH active AS (SELECT * FROM users WHERE active = 1) SELECT * FROM active";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_preserves_whitespace() {
        let sql =
            "SELECT\n    user_id,\n    COUNT(*) AS count\nFROM events\nWHERE event_type = 'click'";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_preserves_doubled_quote_in_string_literal() {
        // SQL standard '' escape inside a single-quoted string must round-trip
        // verbatim so DuckDB sees the same literal it was given.
        let sql = "SELECT CASE WHEN x > 0 THEN 'Can''t Lose Them' ELSE 'Other' END AS label FROM t";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    // ===== Ref resolution tests =====

    #[test]
    fn test_ref_resolution() {
        let sql = "SELECT * FROM smelt.models.users";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM main.users");
    }

    #[test]
    fn test_ref_resolution_custom_schema() {
        let sql = "SELECT * FROM smelt.models.users";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "analytics");
        assert_eq!(result, "SELECT * FROM analytics.users");
    }

    #[test]
    fn test_multiple_refs() {
        let sql = "SELECT a.id, b.id FROM smelt.models.model_a a JOIN smelt.models.model_b b ON a.id = b.id";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("main.model_a"));
        assert!(result.contains("main.model_b"));
        assert!(!result.contains("smelt.ref"));
    }

    // ===== Cross-engine ref resolution tests =====

    #[test]
    fn test_cross_engine_ref_resolution() {
        let sql = "SELECT * FROM smelt.models.spark_model";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let mut cross_refs = HashMap::new();
        cross_refs.insert(
            "spark_model".to_string(),
            "read_parquet('/data/warehouse/default/spark_model/**/*.parquet', hive_partitioning=true)".to_string(),
        );
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: cross_refs,
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        let result = print(&parsed.syntax(), &ctx);
        assert!(
            result.contains("read_parquet("),
            "Expected read_parquet, got: {}",
            result
        );
        assert!(
            result.contains("spark_model/**/*.parquet"),
            "Expected parquet glob path, got: {}",
            result
        );
        assert!(
            !result.contains("main.spark_model"),
            "Should not contain schema-qualified ref, got: {}",
            result
        );
    }

    #[test]
    fn test_cross_engine_ref_mixed_with_normal_refs() {
        let sql = "SELECT a.id, b.id FROM smelt.models.local_model a JOIN smelt.models.spark_model b ON a.id = b.id";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let mut cross_refs = HashMap::new();
        cross_refs.insert(
            "spark_model".to_string(),
            "read_parquet('/data/spark_model/**/*.parquet', hive_partitioning=true)".to_string(),
        );
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: cross_refs,
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        let result = print(&parsed.syntax(), &ctx);
        assert!(
            result.contains("main.local_model"),
            "Normal ref should resolve to schema.model, got: {}",
            result
        );
        assert!(
            result.contains("read_parquet("),
            "Cross-engine ref should resolve to read_parquet, got: {}",
            result
        );
    }

    // ===== Source resolution tests =====

    #[test]
    fn test_source_resolution() {
        let sql = "SELECT * FROM smelt.sources.raw.events";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM raw.events");
    }

    // ===== Formatting/whitespace preservation =====

    #[test]
    fn test_ref_preserves_surrounding_formatting() {
        let sql = "SELECT\n    user_id,\n    COUNT(*) as count\nFROM smelt.models.events\nWHERE event_type = 'click'";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("SELECT\n    user_id,"));
        assert!(result.contains("FROM main.events"));
        assert!(result.contains("\nWHERE event_type = 'click'"));
    }

    // ===== QUALIFY rewrite tests =====

    #[test]
    fn test_qualify_rewrite_postgresql() {
        let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("SELECT * FROM ("),
            "Expected subquery wrapper, got: {}",
            result
        );
        assert!(
            result.contains("WHERE rn = 1"),
            "Expected WHERE clause, got: {}",
            result
        );
        assert!(
            !result.contains("QUALIFY"),
            "QUALIFY should be removed, got: {}",
            result
        );
    }

    #[test]
    fn test_qualify_no_rewrite_duckdb() {
        let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("QUALIFY"), "DuckDB should preserve QUALIFY");
        assert_eq!(result, sql);
    }

    // ===== ARRAY literal rewrite tests =====

    #[test]
    fn test_array_rewrite_spark() {
        let sql = "SELECT ARRAY[1, 2, 3] FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("ARRAY(1, 2, 3)"),
            "Expected ARRAY() syntax, got: {}",
            result
        );
        assert!(
            !result.contains('['),
            "Brackets should be replaced, got: {}",
            result
        );
    }

    #[test]
    fn test_array_no_rewrite_duckdb() {
        let sql = "SELECT ARRAY[1, 2, 3] FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== DATE literal rewrite tests =====

    #[test]
    fn test_date_rewrite_spark() {
        let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("DATE('2024-01-01')"),
            "Expected DATE() function syntax, got: {}",
            result
        );
    }

    #[test]
    fn test_date_no_rewrite_duckdb() {
        let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== :: cast rewrite tests =====

    #[test]
    fn test_double_colon_rewrite_spark() {
        let sql = "SELECT x::INTEGER FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT CAST(x AS INTEGER) FROM t");
    }

    #[test]
    fn test_double_colon_no_rewrite_duckdb() {
        let sql = "SELECT x::INTEGER FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_cast_function_passthrough_spark() {
        let sql = "SELECT CAST(x AS INTEGER) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_double_colon_varchar_rewrite_spark() {
        let sql = "SELECT name::VARCHAR FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT CAST(name AS VARCHAR) FROM t");
    }

    // ===== Trailing comma removal tests =====

    #[test]
    fn test_trailing_comma_stripped_spark() {
        let sql = "SELECT a, b, FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        // The comma is removed; whitespace around it is preserved
        assert!(!result.contains("b,"), "Trailing comma should be removed");
        assert!(result.contains("a, b"), "Non-trailing commas preserved");
        assert!(!result.contains(", FROM"), "Comma before FROM removed");
    }

    #[test]
    fn test_trailing_comma_preserved_duckdb() {
        let sql = "SELECT a, b, FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_group_by_trailing_comma_stripped_spark() {
        let sql = "SELECT a, COUNT(*) FROM t GROUP BY a,";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT a, COUNT(*) FROM t GROUP BY a");
    }

    #[test]
    fn test_no_trailing_comma_unchanged_spark() {
        let sql = "SELECT a, b FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== EXPLODE/UNNEST renaming tests =====

    #[test]
    fn test_explode_to_unnest_duckdb() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT UNNEST(arr) FROM t");
    }

    #[test]
    fn test_unnest_to_explode_spark() {
        let sql = "SELECT UNNEST(arr) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT EXPLODE(arr) FROM t");
    }

    #[test]
    fn test_explode_unchanged_spark() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_unnest_unchanged_duckdb() {
        let sql = "SELECT UNNEST(arr) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_explode_to_unnest_postgresql() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT UNNEST(arr) FROM t");
    }

    // ===== EVERY/BOOL_AND/BOOL_OR remapping tests =====

    #[test]
    fn test_every_to_bool_and_duckdb() {
        let sql = "SELECT EVERY(b) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT BOOL_AND(b) FROM t");
    }

    #[test]
    fn test_every_unchanged_postgresql() {
        // PostgreSQL natively supports EVERY — no remapping needed
        let sql = "SELECT EVERY(b) FROM t";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_bool_and_to_every_spark() {
        let sql = "SELECT BOOL_AND(b) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT EVERY(b) FROM t");
    }

    #[test]
    fn test_bool_or_to_some_spark() {
        let sql = "SELECT BOOL_OR(b) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT SOME(b) FROM t");
    }

    #[test]
    fn test_bool_and_unchanged_duckdb() {
        let sql = "SELECT BOOL_AND(b) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }
}
