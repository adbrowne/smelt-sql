pub mod temporal;

use serde::Serialize;
use smelt_types::SqlFunction;

/// Classification of a SELECT item for optimization analysis.
#[derive(Debug, Clone, Serialize)]
pub enum SelectItemKind {
    /// COUNT(DISTINCT expr) with alias
    CountDistinct { argument: String, alias: String },
    /// Other aggregate function (COUNT(*), SUM, AVG, etc.) with alias
    OtherAggregate { text: String, alias: String },
    /// Non-aggregate expression (GROUP BY key) with alias
    GroupByKey { text: String, alias: String },
}

/// Analyzed structure of a SELECT statement.
#[derive(Debug, Clone, Serialize)]
pub struct SelectAnalysis {
    pub items: Vec<SelectItemKind>,
    /// The FROM clause text (verbatim).
    pub from_text: String,
    /// The WHERE clause text (if present), without the WHERE keyword.
    pub where_text: Option<String>,
    /// GROUP BY expressions (resolved from ordinals if needed).
    pub group_by_exprs: Vec<String>,
    /// Whether a `-- smelt:cube_split` comment was found.
    pub has_cube_split_annotation: bool,
}

/// Analyze a SELECT statement from SQL text.
///
/// Parses the SQL (after stripping frontmatter) and extracts structure
/// needed for optimization decisions.
pub fn analyze_select(sql: &str) -> Option<SelectAnalysis> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let root = parse.syntax();
    let file = smelt_parser::File::cast(root)?;
    let select = file.select_stmt()?;

    // Extract select items with classification
    let select_list = select.select_list()?;
    let mut items = Vec::new();
    let mut select_item_exprs: Vec<String> = Vec::new();

    for item in select_list.items() {
        let expr = item.expression()?;
        let alias = item.column_name().unwrap_or_default();
        let expr_text = expr.text().trim().to_string();
        select_item_exprs.push(expr_text.clone());

        if let Some(func) = expr.as_function_call() {
            let name = func.name().unwrap_or_default().to_uppercase();
            if SqlFunction::from_name(&name) == Some(SqlFunction::Count)
                && has_distinct_keyword(&func)
            {
                let arg = extract_distinct_argument(&func);
                items.push(SelectItemKind::CountDistinct {
                    argument: arg,
                    alias,
                });
                continue;
            }
            // Check if it's any aggregate function
            if SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate()) {
                items.push(SelectItemKind::OtherAggregate {
                    text: expr_text,
                    alias,
                });
                continue;
            }
        }

        items.push(SelectItemKind::GroupByKey {
            text: expr_text,
            alias,
        });
    }

    // Extract FROM clause text
    let from_clause = select.from_clause()?;
    let from_text = from_clause.text();

    // Extract WHERE clause text (without WHERE keyword)
    let where_text = select.where_clause().map(|w| {
        let full = w.text();
        // Strip the leading "WHERE" keyword
        let stripped = full.trim_start();
        if let Some(rest) = stripped.strip_prefix("WHERE") {
            rest.trim().to_string()
        } else if let Some(rest) = stripped.strip_prefix("where") {
            rest.trim().to_string()
        } else {
            full
        }
    });

    // Extract GROUP BY expressions from raw text, resolving ordinals
    let group_by_exprs = extract_group_by_from_text(stripped, &select_item_exprs);

    // Check for smelt:cube_split annotation in comments
    let has_cube_split_annotation = check_cube_split_annotation(stripped);

    Some(SelectAnalysis {
        items,
        from_text,
        where_text,
        group_by_exprs,
        has_cube_split_annotation,
    })
}

/// Check if a FunctionCall contains the DISTINCT keyword by examining its text.
fn has_distinct_keyword(func: &smelt_parser::FunctionCall) -> bool {
    // Use the function's text representation and check for DISTINCT
    let text = func.text().to_uppercase();
    // Match COUNT(DISTINCT ...) pattern
    text.contains("DISTINCT")
}

/// Extract the argument expression text from COUNT(DISTINCT expr).
fn extract_distinct_argument(func: &smelt_parser::FunctionCall) -> String {
    let args = func.arguments();
    if let Some(first) = args.first() {
        first.text().trim().to_string()
    } else {
        String::new()
    }
}

/// Extract GROUP BY expressions from raw SQL text, resolving ordinal references.
///
/// This avoids needing access to parser internals for GROUP BY clause nodes.
fn extract_group_by_from_text(sql: &str, select_item_exprs: &[String]) -> Vec<String> {
    let upper = sql.to_uppercase();

    // Find "GROUP BY" in the SQL
    let group_by_pos = match upper.find("GROUP BY") {
        Some(pos) => pos,
        None => return Vec::new(),
    };

    let after_group_by = &sql[group_by_pos + 8..]; // Skip "GROUP BY"

    // Find where GROUP BY clause ends (at HAVING, ORDER BY, LIMIT, QUALIFY, UNION, or end/comment)
    let end_keywords = [
        "HAVING",
        "ORDER",
        "LIMIT",
        "QUALIFY",
        "UNION",
        "INTERSECT",
        "EXCEPT",
        "FETCH",
    ];
    let after_upper = after_group_by.to_uppercase();
    let mut end_pos = after_group_by.len();

    for kw in &end_keywords {
        if let Some(pos) = find_keyword_not_in_parens(&after_upper, kw) {
            if pos < end_pos {
                end_pos = pos;
            }
        }
    }

    // Also stop at line comment
    if let Some(comment_pos) = after_group_by.find("--") {
        if comment_pos < end_pos {
            end_pos = comment_pos;
        }
    }

    let group_by_text = &after_group_by[..end_pos];

    // Split by commas (not inside parentheses)
    let exprs = split_by_comma_not_in_parens(group_by_text);

    exprs
        .into_iter()
        .map(|expr| {
            let trimmed = expr.trim().to_string();
            // Resolve ordinal references
            if let Ok(ordinal) = trimmed.parse::<usize>() {
                if ordinal > 0 && ordinal <= select_item_exprs.len() {
                    return select_item_exprs[ordinal - 1].clone();
                }
            }
            trimmed
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Find a keyword in text that's not inside parentheses.
fn find_keyword_not_in_parens(text: &str, keyword: &str) -> Option<usize> {
    let mut depth = 0;
    let bytes = text.as_bytes();
    let kw_bytes = keyword.as_bytes();

    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0
            && i + kw_bytes.len() <= bytes.len()
            && &bytes[i..i + kw_bytes.len()] == kw_bytes
        {
            // Check word boundary
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = i + kw_bytes.len() >= bytes.len()
                || !bytes[i + kw_bytes.len()].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

/// Split a string by commas, ignoring commas inside parentheses.
fn split_by_comma_not_in_parens(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in text.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current);
    }

    parts
}

/// Check for `-- smelt:cube_split` comment anywhere in the SQL.
fn check_cube_split_annotation(sql: &str) -> bool {
    for line in sql.lines() {
        if let Some(comment_start) = line.find("--") {
            let comment = &line[comment_start..];
            if comment.contains("smelt:cube_split") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_basic_select() {
        let sql = "SELECT country, COUNT(DISTINCT user_id) as unique_users FROM events GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.items.len(), 2);
        assert!(
            matches!(&analysis.items[0], SelectItemKind::GroupByKey { text, .. } if text == "country")
        );
        assert!(
            matches!(&analysis.items[1], SelectItemKind::CountDistinct { argument, alias, .. } if argument == "user_id" && alias == "unique_users")
        );
        assert_eq!(analysis.group_by_exprs, vec!["country"]);
    }

    #[test]
    fn test_analyze_multiple_count_distinct() {
        let sql = r#"
            SELECT
                date_trunc('day', event_time) as event_date,
                country,
                COUNT(DISTINCT user_id) as unique_users,
                COUNT(DISTINCT session_id) as unique_sessions
            FROM events
            GROUP BY 1, 2
        "#;
        let analysis = analyze_select(sql).unwrap();

        let count_distincts: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::CountDistinct { .. }))
            .collect();
        assert_eq!(count_distincts.len(), 2);

        let group_keys: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::GroupByKey { .. }))
            .collect();
        assert_eq!(group_keys.len(), 2);
    }

    #[test]
    fn test_cube_split_annotation_detected() {
        let sql = "SELECT a, COUNT(DISTINCT b) as cb FROM t GROUP BY 1 -- smelt:cube_split";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.has_cube_split_annotation);
    }

    #[test]
    fn test_no_cube_split_annotation() {
        let sql = "SELECT a, COUNT(DISTINCT b) as cb FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(!analysis.has_cube_split_annotation);
    }

    #[test]
    fn test_analyze_with_where_clause() {
        let sql = "SELECT a, COUNT(*) as cnt FROM t WHERE active = true GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.where_text.is_some());
        let where_text = analysis.where_text.unwrap();
        assert!(where_text.contains("active"));
    }

    #[test]
    fn test_other_aggregates() {
        let sql = "SELECT country, COUNT(*) as cnt, SUM(revenue) as total FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();

        let others: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::OtherAggregate { .. }))
            .collect();
        assert_eq!(others.len(), 2);
    }

    #[test]
    fn test_ordinal_resolution() {
        let sql = "SELECT country, city, COUNT(DISTINCT user_id) as users FROM t GROUP BY 1, 2";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.group_by_exprs.len(), 2);
        assert_eq!(analysis.group_by_exprs[0], "country");
        assert_eq!(analysis.group_by_exprs[1], "city");
    }

    #[test]
    fn test_from_text_preserved() {
        let sql = "SELECT a FROM smelt.models.events e GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.from_text.contains("smelt.models.events"));
    }

    #[test]
    fn test_frontmatter_stripped() {
        let sql = "---\nmaterialized: table\n---\nSELECT a FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.items.len(), 1);
    }
}
