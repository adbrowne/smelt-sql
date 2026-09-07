use serde::Serialize;
use smelt_types::SqlFunction;

use crate::analysis::partition_alignment::group_by_all_keys;

/// Classification of a SELECT item for optimization analysis.
#[derive(Debug, Clone, Serialize)]
pub enum SelectItemKind {
    /// COUNT(DISTINCT expr) with alias
    CountDistinct {
        argument: String,
        alias: String,
        /// The parsed argument expression, retained so downstream analyses
        /// (e.g. the event-time monotonicity trace) never re-parse.
        #[serde(skip)]
        expr: smelt_parser::Expr,
    },
    /// Other aggregate function (COUNT(*), SUM, AVG, etc.) with alias
    OtherAggregate {
        text: String,
        alias: String,
        /// The parsed select-item expression, retained so downstream
        /// analyses never re-parse.
        #[serde(skip)]
        expr: smelt_parser::Expr,
    },
    /// Non-aggregate expression (GROUP BY key) with alias
    GroupByKey {
        text: String,
        alias: String,
        /// The parsed select-item expression, retained so downstream
        /// analyses (e.g. the event-time monotonicity trace) never re-parse.
        #[serde(skip)]
        expr: smelt_parser::Expr,
    },
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
    /// Clauses present on the SELECT that a textual reassembly of
    /// (items, FROM, WHERE, GROUP BY) would silently drop. Consumers that
    /// rebuild the query from those parts (e.g. the cube_split rewrite)
    /// must refuse when any of these is set.
    pub unreconstructible_clauses: Vec<&'static str>,
}

/// Classify the items of an already-parsed `SelectList` into `SelectItemKind`s.
///
/// Factored out of [`analyze_select`] so downstream analyses that already
/// hold a parsed `smelt_parser::SelectStmt` — a UNION branch, a subquery
/// body, a CTE body — can classify its items without a second text-level
/// parse (the event-time monotonicity trace consumers added in this phase;
/// see `rules::incremental`).
pub fn classify_select_items(
    select_list: &smelt_parser::SelectList,
) -> Option<Vec<SelectItemKind>> {
    let mut items = Vec::new();
    for item in select_list.items() {
        let expr = item.expression()?;
        let alias = item.column_name().unwrap_or_default();
        let expr_text = expr.text().trim().to_string();

        if let Some(func) = expr.as_function_call() {
            let name = func.name().unwrap_or_default().to_uppercase();
            if SqlFunction::from_name(&name) == Some(SqlFunction::Count)
                && has_distinct_keyword(&func)
            {
                let arg = extract_distinct_argument(&func);
                items.push(SelectItemKind::CountDistinct {
                    argument: arg,
                    alias,
                    expr: expr.clone(),
                });
                continue;
            }
            // Check if it's any aggregate function
            if SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate()) {
                items.push(SelectItemKind::OtherAggregate {
                    text: expr_text,
                    alias,
                    expr: expr.clone(),
                });
                continue;
            }
        }

        // Window-function projections (`ROW_NUMBER() OVER (...)`, etc.) are
        // neither an aggregate nor a plain grouping expression: DuckDB never
        // treats a window item as a `GROUP BY ALL` key. Route it to the
        // non-key `OtherAggregate` kind so every consumer of grouping keys
        // (`group_by_all_keys` and friends) excludes it, the same way it
        // already excludes real aggregates.
        if expr.window_spec().is_some() {
            items.push(SelectItemKind::OtherAggregate {
                text: expr_text,
                alias,
                expr: expr.clone(),
            });
            continue;
        }

        items.push(SelectItemKind::GroupByKey {
            text: expr_text,
            alias,
            expr: expr.clone(),
        });
    }
    Some(items)
}

/// Classify the items of a parsed `SelectStmt` (any branch — outer query,
/// UNION branch, subquery/CTE body). Returns `None` if the statement has no
/// SELECT list (should not happen for a well-formed parse).
pub fn select_stmt_items(select: &smelt_parser::SelectStmt) -> Option<Vec<SelectItemKind>> {
    let select_list = select.select_list()?;
    classify_select_items(&select_list)
}

/// The alias of a classified select item.
pub fn item_alias(item: &SelectItemKind) -> &str {
    match item {
        SelectItemKind::CountDistinct { alias, .. } => alias,
        SelectItemKind::OtherAggregate { alias, .. } => alias,
        SelectItemKind::GroupByKey { alias, .. } => alias,
    }
}

/// The parsed expression of a classified select item.
pub fn item_expr(item: &SelectItemKind) -> &smelt_parser::Expr {
    match item {
        SelectItemKind::CountDistinct { expr, .. } => expr,
        SelectItemKind::OtherAggregate { expr, .. } => expr,
        SelectItemKind::GroupByKey { expr, .. } => expr,
    }
}

/// Find the expression of the item aliased `alias`. Falls back to matching
/// by ordinal `position` (0-based) among `items` when no alias matches —
/// used for UNION branches / subquery bodies whose SELECT list does not
/// repeat the outer alias (SQL takes UNION output column names from the
/// first branch only).
pub fn find_item_expr_by_alias_or_position(
    items: &[SelectItemKind],
    alias: &str,
    position: usize,
) -> Option<smelt_parser::Expr> {
    items
        .iter()
        .find(|item| item_alias(item) == alias)
        .or_else(|| items.get(position))
        .map(item_expr)
        .cloned()
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
    let items = classify_select_items(&select_list)?;
    let select_item_exprs: Vec<String> = items
        .iter()
        .map(|item| item_expr(item).text().trim().to_string())
        .collect();

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

    // Extract GROUP BY expressions. `GROUP BY ALL` (DuckDB) carries no explicit
    // key expressions — expand it to the non-aggregate select items via the
    // shared leaf classifier so it is analyzed identically to its explicit
    // twin, never as a phantom `["ALL"]` key. The `is_all()` check reads the
    // parsed clause marker, not a raw-text scan.
    let group_by_exprs = if select.group_by_clause().is_some_and(|gb| gb.is_all()) {
        group_by_all_keys(&items)
    } else {
        extract_group_by_from_text(stripped, &select_item_exprs)
    };

    // Check for smelt:cube_split annotation in comments
    let has_cube_split_annotation = check_cube_split_annotation(stripped);

    let mut unreconstructible_clauses = Vec::new();
    if select.is_distinct() {
        unreconstructible_clauses.push("DISTINCT");
    }
    if select.having_clause().is_some() {
        unreconstructible_clauses.push("HAVING");
    }
    if select.qualify_clause().is_some() {
        unreconstructible_clauses.push("QUALIFY");
    }
    if select.order_by_clause().is_some() {
        unreconstructible_clauses.push("ORDER BY");
    }
    if select.limit_clause().is_some() {
        unreconstructible_clauses.push("LIMIT");
    }

    Some(SelectAnalysis {
        items,
        from_text,
        where_text,
        group_by_exprs,
        has_cube_split_annotation,
        unreconstructible_clauses,
    })
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): scoped to one already-resolved function call's own text, not the
/// surrounding query — safe to compose under the walk.
///
/// Check if a FunctionCall contains the DISTINCT keyword by examining its text.
pub fn has_distinct_keyword(func: &smelt_parser::FunctionCall) -> bool {
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

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): locates one clause's boundary within the outer scope's own text so
/// the caller ([`extract_group_by_from_text`]) can extract a leaf-level GROUP
/// BY expression list; it never reasons about a nested scope's structure.
///
/// Find the byte-position of the last `GROUP BY` keyword in `sql` that is not
/// inside a line comment (`--`).  Returns `None` if no such occurrence exists.
///
/// We look for the *last* occurrence so that a comment earlier in the SQL (e.g.
/// `-- … GROUP BY …`) does not shadow the real GROUP BY clause at the end.
fn find_group_by_outside_comments(sql: &str) -> Option<usize> {
    let upper = sql.to_uppercase();
    let keyword = "GROUP BY";
    let kw_len = keyword.len();
    let mut best: Option<usize> = None;

    let mut start = 0;
    while let Some(pos) = upper[start..].find(keyword) {
        let abs_pos = start + pos;

        // Check whether this occurrence is inside a line comment by scanning
        // back to the start of the current line.
        let line_start = sql[..abs_pos].rfind('\n').map_or(0, |p| p + 1);
        let line_before = &sql[line_start..abs_pos];
        if !line_before.contains("--") {
            // Not in a comment — record as candidate.
            best = Some(abs_pos);
        }

        start = abs_pos + kw_len;
    }

    best
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): extracts the outer scope's own GROUP BY expression list from its
/// own text; it does not reason about nested scopes.
///
/// Extract GROUP BY expressions from raw SQL text, resolving ordinal references.
///
/// This avoids needing access to parser internals for GROUP BY clause nodes.
fn extract_group_by_from_text(sql: &str, select_item_exprs: &[String]) -> Vec<String> {
    // Find "GROUP BY" in the SQL, skipping occurrences inside line comments.
    let group_by_pos = match find_group_by_outside_comments(sql) {
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

/// True for the characters that can appear inside a bare identifier — an
/// end-keyword boundary check must treat `_` as identifier-forming (not just
/// alphanumeric), or a column named e.g. `order_id` collides with the `ORDER`
/// end-keyword mid-identifier (`_` before "non-alphanumeric" == false boundary).
fn is_identifier_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
            // Word boundary: neither neighbor may be an identifier char, and
            // the keyword may not be a quoted/qualified identifier (`t.order`,
            // `"order"`, a backtick-quoted name).
            let before_ok = i == 0
                || (!is_identifier_char(bytes[i - 1])
                    && bytes[i - 1] != b'.'
                    && bytes[i - 1] != b'"'
                    && bytes[i - 1] != b'`');
            let after_ok = i + kw_bytes.len() >= bytes.len()
                || (!is_identifier_char(bytes[i + kw_bytes.len()])
                    && bytes[i + kw_bytes.len()] != b'"'
                    && bytes[i + kw_bytes.len()] != b'`');
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

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): a model-level presentation annotation lookup, not a composition
/// verdict — reads the whole model's comments but never feeds bound/reach,
/// admission, or monotonicity derivation.
///
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
