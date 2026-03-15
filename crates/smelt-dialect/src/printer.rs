//! Dialect-aware CST printer.
//!
//! Walks a Rowan CST and emits SQL text, performing dialect-specific rewrites:
//! - `smelt.ref('model')` → `schema.model`
//! - `smelt.source('raw.events')` → `raw.events`
//! - QUALIFY → subquery rewrite (when `!caps.supports_qualify`)
//! - ARRAY[1,2,3] → ARRAY(1,2,3) (when `!caps.supports_array_literal`)
//! - DATE '2024-01-01' → DATE('2024-01-01') (when `!caps.supports_date_literal`)
//! - expr::type → CAST(expr AS type) (when `!caps.supports_double_colon_cast`)
//!
//! The default behavior is verbatim: tokens (including whitespace and comments)
//! are emitted exactly as they appear in the source. This guarantees an identity
//! property for DuckDB with no refs/sources.

use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::{CastExpr, FunctionCall, RefCall, SourceCall};

use crate::{BackendCapabilities, SqlDialect};

/// Context for dialect-aware printing.
pub struct PrintContext<'a> {
    pub dialect: &'a SqlDialect,
    pub capabilities: &'a BackendCapabilities,
    pub schema: &'a str,
}

/// Print a CST node as dialect-specific SQL.
pub fn print(node: &SyntaxNode, ctx: &PrintContext) -> String {
    let mut out = String::new();
    print_node(node, ctx, &mut out);
    out
}

fn print_node(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    match node.kind() {
        SyntaxKind::FUNCTION_CALL => {
            if let Some(fc) = FunctionCall::cast(node.clone()) {
                if let Some(ref_call) = RefCall::from_function_call(fc.clone()) {
                    if let Some(model_name) = ref_call.model_name() {
                        out.push_str(ctx.schema);
                        out.push('.');
                        out.push_str(&model_name);
                        return;
                    }
                }
                if let Some(source_call) = SourceCall::from_function_call(fc.clone()) {
                    if let Some(qualified) = source_call.qualified_name() {
                        out.push_str(&qualified);
                        return;
                    }
                }
                // EXPLODE/UNNEST renaming
                if let Some(name) = fc.name() {
                    let new_name = match ctx.dialect {
                        SqlDialect::DuckDB | SqlDialect::PostgreSQL => {
                            if name.eq_ignore_ascii_case("EXPLODE") {
                                Some("UNNEST")
                            } else {
                                None
                            }
                        }
                        SqlDialect::SparkSQL => {
                            if name.eq_ignore_ascii_case("UNNEST") {
                                Some("EXPLODE")
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(new_name) = new_name {
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

    // ===== Ref resolution tests =====

    #[test]
    fn test_ref_resolution() {
        let sql = "SELECT * FROM smelt.ref('users')";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM main.users");
    }

    #[test]
    fn test_ref_resolution_custom_schema() {
        let sql = "SELECT * FROM smelt.ref('users')";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "analytics");
        assert_eq!(result, "SELECT * FROM analytics.users");
    }

    #[test]
    fn test_multiple_refs() {
        let sql = "SELECT a.id, b.id FROM smelt.ref('model_a') a JOIN smelt.ref('model_b') b ON a.id = b.id";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("main.model_a"));
        assert!(result.contains("main.model_b"));
        assert!(!result.contains("smelt.ref"));
    }

    // ===== Source resolution tests =====

    #[test]
    fn test_source_resolution() {
        let sql = "SELECT * FROM smelt.source('raw.events')";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM raw.events");
    }

    // ===== Formatting/whitespace preservation =====

    #[test]
    fn test_ref_preserves_surrounding_formatting() {
        let sql = "SELECT\n    user_id,\n    COUNT(*) as count\nFROM smelt.ref('events')\nWHERE event_type = 'click'";
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
}
