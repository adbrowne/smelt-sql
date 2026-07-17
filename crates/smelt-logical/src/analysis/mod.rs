pub mod bounded_domain;
pub mod decomposed_state;
pub mod discriminants;
pub mod functional_dependency;
pub mod horizon_ceiling;
pub mod input_delta;
pub mod join_shape;
pub mod model_diff;
pub mod monotonicity;
pub mod not_null;
pub mod presentation;
pub mod source_bounds;
pub mod temporal;
pub mod walk;
pub mod window_independence;

pub use walk::{
    enumerate_scopes, model_property_vector, walk, ColumnDeterminism, ColumnDiscriminant,
    ColumnLineage, CteDef, DerivedFd, Determinism, Grain, InputItem, KeySet, LeafColumn, LeafInput,
    NodeCx, OpNode, PathSeg, PropertyTransfer, PropertyVector, QueryNode, QueryTree,
    RelationSource, Scope, ScopeEnum, ScopeEnumeration, ScopeKind, SelectNode, SetOpKind,
    SetOpNode, Transfer, UnsupportedConstruct,
};

use serde::Serialize;
use smelt_types::SqlFunction;

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

/// The verdict for whether a SELECT scope's own `GROUP BY` / `DISTINCT` key
/// is a superset of the model's `partition_column` — the shared
/// partition-alignment signal (`incremental_models.md` §"Safety checks") that
/// licenses group-aligned `HAVING`/`DISTINCT` admission
/// (`rules::incremental`) and is available to other per-scope consumers
/// (UNION-branch / window admission) as the same reusable check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionAlignment {
    /// The scope's own key is a superset of `partition_column` — safe.
    Aligned,
    /// The scope's own key does not contain `partition_column`; `reason`
    /// names why (no `GROUP BY` in this scope, `partition_column` not
    /// projected here, or the key omits it).
    NotAligned { reason: String },
}

impl PartitionAlignment {
    pub fn is_aligned(&self) -> bool {
        matches!(self, PartitionAlignment::Aligned)
    }
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): the DuckDB `GROUP BY ALL` expansion — every non-aggregate select
/// item of this one already-classified scope becomes a grouping key, mirroring
/// DuckDB's own semantics. It reasons only over the caller's already-bounded
/// `items`, never over raw SQL text, so it is the single source of expansion
/// truth shared by both the AST path ([`resolve_scope_group_by`]) and the
/// text-scan path ([`analyze_select`]).
fn group_by_all_keys(items: &[SelectItemKind]) -> Vec<String> {
    items
        .iter()
        .filter(|item| matches!(item, SelectItemKind::GroupByKey { .. }))
        .map(|item| item_expr(item).text().trim().to_string())
        .collect()
}

/// Resolve `select`'s own `GROUP BY` expressions against **its own**
/// select-list `items` — ordinal references (`GROUP BY 1, 2`) resolve to
/// this scope's projections, not an outer query's. `GROUP BY ALL` expands to
/// the scope's non-aggregate select items (DuckDB semantics). Returns an empty
/// `Vec` when the scope has no `GROUP BY` clause (or a `GROUP BY ALL` over an
/// aggregates-only projection — single-group semantics).
pub fn resolve_scope_group_by(
    select: &smelt_parser::SelectStmt,
    items: &[SelectItemKind],
) -> Vec<String> {
    let Some(group_by) = select.group_by_clause() else {
        return Vec::new();
    };
    // DuckDB `GROUP BY ALL`: the clause carries no explicit key expressions —
    // it groups by every non-aggregate select item. Expand to those items'
    // expression text so the walk sees the real grouping keys (never a phantom
    // `ALL` key). An aggregates-only projection yields an empty key set, i.e.
    // single-group semantics, identical to a plain aggregate without GROUP BY.
    if group_by.is_all() {
        return group_by_all_keys(items);
    }
    group_by
        .expressions()
        .map(|expr| {
            let text = expr.text().trim().to_string();
            if let Ok(ordinal) = text.parse::<usize>() {
                if ordinal >= 1 {
                    if let Some(item) = items.get(ordinal - 1) {
                        return item_expr(item).text().trim().to_string();
                    }
                }
            }
            text
        })
        .collect()
}

/// Partition-alignment verdict for a `HAVING` clause living in `select`:
/// `Aligned` when this scope's own resolved `GROUP BY` keys are a superset
/// containing the projected `partition_col` expression — found among
/// `select`'s **own** select-list items, so a subquery/UNION-branch body is
/// judged by its own projections and its own `GROUP BY`, never the outer
/// query's (`incremental_models.md` §"Safety checks").
pub fn scope_group_by_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    let Some(partition_item) = items.iter().find(|item| item_alias(item) == partition_col) else {
        return PartitionAlignment::NotAligned {
            reason: format!("partition_column '{partition_col}' is not projected in this scope"),
        };
    };
    let partition_expr = item_expr(partition_item).text().trim().to_string();
    let group_by_keys = resolve_scope_group_by(select, &items);
    if group_by_keys.is_empty() {
        return PartitionAlignment::NotAligned {
            reason: "this scope has no GROUP BY".to_string(),
        };
    }
    if group_by_keys.iter().any(|k| k == &partition_expr) {
        PartitionAlignment::Aligned
    } else {
        PartitionAlignment::NotAligned {
            reason: format!(
                "GROUP BY ({}) does not include the partition_column '{partition_col}' expression '{partition_expr}'",
                group_by_keys.join(", ")
            ),
        }
    }
}

/// Partition-alignment verdict for a `SELECT DISTINCT` living in `select`:
/// the dedup key is the whole projected row, so it is `Aligned` whenever
/// `partition_col` is itself projected in this scope's own select list
/// (`timeseries.md`'s partition-column-projection rule already requires
/// this at the outer scope; checked independently here so an inner scope
/// that does *not* project `partition_col` is not erroneously admitted).
pub fn scope_distinct_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    if items.iter().any(|item| item_alias(item) == partition_col) {
        PartitionAlignment::Aligned
    } else {
        PartitionAlignment::NotAligned {
            reason: format!("partition_column '{partition_col}' is not projected in this scope"),
        }
    }
}

/// Partition-alignment verdict for the window `OVER` scope(s) living in
/// `select`'s own select list: `Aligned` when **every** window found there
/// has a `PARTITION BY` whose keys are a superset containing `partition_col`
/// (AST-based, replacing the substring `OVER`/`PARTITION BY` scan). A scope
/// with no window at all, or any window missing `PARTITION BY` or omitting
/// `partition_col`, fails closed to `NotAligned{reason}` — never optimistic.
/// Judged against **this** scope's own select list only (a FROM subquery's
/// window is read by resolving to that subquery's own `SelectStmt` first).
pub fn scope_over_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    let windows: Vec<smelt_parser::WindowSpec> = items
        .iter()
        .filter_map(|item| item_expr(item).window_spec())
        .collect();
    if windows.is_empty() {
        return PartitionAlignment::NotAligned {
            reason: "this scope has no window OVER clause".to_string(),
        };
    }
    for window in &windows {
        if let not_aligned @ PartitionAlignment::NotAligned { .. } =
            window_over_alignment(window, partition_col)
        {
            return not_aligned;
        }
    }
    PartitionAlignment::Aligned
}

/// Partition-alignment verdict for a single window `OVER` clause: `Aligned`
/// when its `PARTITION BY` keys are a superset containing `partition_col`.
/// A window with no `PARTITION BY` (including a named-window reference,
/// `OVER w`) fails closed to `NotAligned` — never optimistic. This is the
/// per-window leaf classifier the composition walk invokes for every window
/// scope it enumerates (`model_properties.md` §"The composition walk").
pub fn window_over_alignment(
    window: &smelt_parser::WindowSpec,
    partition_col: &str,
) -> PartitionAlignment {
    let Some(partition_by) = window.partition_by() else {
        return PartitionAlignment::NotAligned {
            reason: "a window OVER clause in this scope has no PARTITION BY".to_string(),
        };
    };
    let keys: Vec<String> = partition_by
        .expressions()
        .map(|e| e.text().trim().to_string())
        .collect();
    if !keys.iter().any(|k| k == partition_col) {
        return PartitionAlignment::NotAligned {
            reason: format!(
                "window OVER (PARTITION BY {}) does not include the partition_column '{partition_col}'",
                keys.join(", ")
            ),
        };
    }
    PartitionAlignment::Aligned
}

/// True when a window's frame is a bounded `RANGE BETWEEN INTERVAL '…'
/// PRECEDING [AND …]` (Form A) with no `UNBOUNDED` bound. Such a frame has a
/// finite reach the source-bound deriver picks up and the planner widens the
/// source read to cover, so the window is partition-local up to that bound —
/// admissible even when its `PARTITION BY` omits the partition column
/// (alignment is the zero-lookback license; frame-reach is the
/// bounded-lookback one). An `UNBOUNDED` bound reads across all history and
/// is deliberately excluded. Leaf classifier over the frame clause's AST.
pub fn window_has_bounded_range_interval_frame(window: &smelt_parser::WindowSpec) -> bool {
    let Some(frame) = window.frame() else {
        return false;
    };
    if frame.unit() != Some(smelt_parser::FrameUnit::Range) {
        return false;
    }
    let bounds = frame.bounds();
    // `BETWEEN <lo> AND <hi>` yields two bounds; the single-bound spelling
    // (`RANGE INTERVAL '…' PRECEDING`) is not the derivable Form A shape.
    if bounds.len() != 2 {
        return false;
    }
    let has_interval = bounds
        .iter()
        .any(|b| b.text().to_uppercase().contains("INTERVAL"));
    let has_unbounded = bounds
        .iter()
        .any(|b| b.text().to_uppercase().contains("UNBOUNDED"));
    has_interval && !has_unbounded
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

    Some(SelectAnalysis {
        items,
        from_text,
        where_text,
        group_by_exprs,
        has_cube_split_annotation,
    })
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): scoped to one already-resolved function call's own text, not the
/// surrounding query — safe to compose under the walk.
///
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
    fn analyze_select_retains_expr_for_group_key() {
        let sql = "SELECT DATE_TRUNC('day', ts) AS d, COUNT(*) as cnt FROM events GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        let SelectItemKind::GroupByKey { text, expr, .. } = &analysis.items[0] else {
            panic!("expected GroupByKey item");
        };
        assert_eq!(expr.text().trim(), text.as_str());
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

    #[test]
    fn test_group_by_not_in_comment() {
        // GROUP BY in a line comment must not be confused with the actual GROUP BY.
        // Regression: models with "GROUP BY" in a comment caused extraction to grab
        // the wrong position (comment text instead of the real clause).
        let sql = r#"
            -- session_start_date appears in both the SELECT list and the GROUP BY.
            SELECT
                s.session_id,
                s.session_start_date,
                'u:' || CAST(arg_max(e.user_id, e.event_ts) AS VARCHAR) AS fwd
            FROM sessions s
            GROUP BY s.session_id, s.session_start_date
        "#;
        let analysis = analyze_select(sql).unwrap();
        // Must extract from the real GROUP BY, not the comment.
        assert!(
            analysis
                .group_by_exprs
                .contains(&"s.session_start_date".to_string()),
            "expected s.session_start_date in group_by_exprs; got: {:?}",
            analysis.group_by_exprs
        );
    }

    fn parse_select(sql: &str) -> smelt_parser::SelectStmt {
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        file.select_stmt().expect("select stmt")
    }

    #[test]
    fn test_scope_group_by_alignment_aligned() {
        let select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY event_date, user_id HAVING COUNT(*) > 1",
        );
        assert_eq!(
            scope_group_by_alignment(&select, "event_date"),
            PartitionAlignment::Aligned
        );
    }

    #[test]
    fn test_scope_group_by_alignment_not_aligned_fails_closed() {
        // GROUP BY omits the partition_column entirely.
        let select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY user_id HAVING COUNT(*) > 1",
        );
        assert!(!scope_group_by_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_group_by_alignment_no_group_by_fails_closed() {
        let select = parse_select("SELECT a, b FROM t");
        assert!(!scope_group_by_alignment(&select, "a").is_aligned());
    }

    #[test]
    fn test_grouping_sets_grain_verdict_mirrors_cube_verdict() {
        // Neither `CUBE(...)` nor `GROUPING SETS (...)` has dedicated
        // smelt-side grammar for grain/FD purposes — both flow through
        // `resolve_scope_group_by` as one opaque grouping-key expression
        // whose text is the whole construct (e.g. "CUBE(event_date, user_id)"
        // / "GROUPING SETS ((event_date), (user_id))"). That text never
        // matches a plain projected column name, so both are conservatively
        // judged `NotAligned` — never a phantom `Aligned` claim that
        // `event_date` (or any other column) is a genuine grouping key of
        // this scope. This is the "same verdict class" the GROUPING SETS
        // implementation is required to mirror from the CUBE/ROLLUP
        // precedent (there being no richer precedent to match, since neither
        // gets special-cased grain treatment today).
        let cube_select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY CUBE(event_date, user_id) HAVING COUNT(*) > 1",
        );
        let cube_verdict = scope_group_by_alignment(&cube_select, "event_date");
        assert!(
            !cube_verdict.is_aligned(),
            "CUBE grouping key text never matches a plain column name: {cube_verdict:?}"
        );

        let grouping_sets_select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY GROUPING SETS ((event_date), (user_id)) HAVING COUNT(*) > 1",
        );
        let grouping_sets_verdict = scope_group_by_alignment(&grouping_sets_select, "event_date");
        assert!(
            !grouping_sets_verdict.is_aligned(),
            "GROUPING SETS must mirror CUBE's conservative verdict: {grouping_sets_verdict:?}"
        );

        // Both land in the same verdict variant (`NotAligned`), not merely
        // both "not Aligned" by coincidence of different enum shapes.
        assert!(matches!(
            cube_verdict,
            PartitionAlignment::NotAligned { .. }
        ));
        assert!(matches!(
            grouping_sets_verdict,
            PartitionAlignment::NotAligned { .. }
        ));

        // Sanity: resolve_scope_group_by sees exactly one opaque key for
        // each — the whole construct's own text — confirming there is no
        // phantom expansion into ["event_date", "user_id"].
        let items = select_stmt_items(&cube_select).unwrap_or_default();
        let cube_keys = resolve_scope_group_by(&cube_select, &items);
        assert_eq!(cube_keys.len(), 1);
        assert!(cube_keys[0].to_uppercase().starts_with("CUBE"));

        let gs_items = select_stmt_items(&grouping_sets_select).unwrap_or_default();
        let gs_keys = resolve_scope_group_by(&grouping_sets_select, &gs_items);
        assert_eq!(gs_keys.len(), 1);
        assert!(gs_keys[0].to_uppercase().starts_with("GROUPING SETS"));
    }

    #[test]
    fn test_scope_distinct_alignment_aligned_when_projected() {
        let select = parse_select("SELECT DISTINCT event_date, user_id FROM events");
        assert!(scope_distinct_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_distinct_alignment_not_aligned_when_not_projected() {
        let select = parse_select("SELECT DISTINCT user_id FROM events");
        assert!(!scope_distinct_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_aligned_when_partition_by_superset() {
        let select = parse_select(
            "SELECT event_date, user_id, \
             SUM(amount) OVER (PARTITION BY event_date, user_id ORDER BY user_id) AS running \
             FROM events",
        );
        assert_eq!(
            scope_over_alignment(&select, "event_date"),
            PartitionAlignment::Aligned
        );
    }

    #[test]
    fn test_scope_over_alignment_not_aligned_when_partition_by_omits_column() {
        let select = parse_select(
            "SELECT event_date, user_id, \
             SUM(amount) OVER (PARTITION BY user_id ORDER BY user_id) AS running \
             FROM events",
        );
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_is_per_scope_not_outer() {
        // The outer query has no window at all; the FROM subquery's own
        // window is aligned. Reading the outer scope must not see the
        // subquery's alignment, and reading the subquery's own scope must
        // see it correctly regardless of the outer query's shape.
        let outer = parse_select(
            "SELECT * FROM (\
                 SELECT event_date, user_id, \
                 SUM(amount) OVER (PARTITION BY event_date ORDER BY user_id) AS running \
                 FROM events\
             ) t",
        );
        assert!(
            !scope_over_alignment(&outer, "event_date").is_aligned(),
            "outer scope has no window OVER of its own"
        );

        let inner = outer
            .from_clause()
            .expect("from clause")
            .table_refs()
            .next()
            .expect("table ref")
            .subquery()
            .expect("subquery")
            .select_stmt()
            .expect("inner select");
        assert!(
            scope_over_alignment(&inner, "event_date").is_aligned(),
            "inner scope's own window is partition-aligned"
        );
    }

    #[test]
    fn test_scope_over_alignment_fails_closed_when_no_partition_by() {
        // A window with no PARTITION BY at all must never be optimistically
        // treated as aligned.
        let select = parse_select(
            "SELECT event_date, ROW_NUMBER() OVER (ORDER BY event_date) AS rn FROM events",
        );
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_no_window_fails_closed() {
        let select = parse_select("SELECT event_date, user_id FROM events");
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    /// The alignment verdict is computed **per-scope**: a UNION's second
    /// branch has its own `GROUP BY` (omitting the partition_column), which
    /// must be judged on its own terms — not the first branch's (aligned)
    /// `GROUP BY`.
    #[test]
    fn test_alignment_is_per_scope_not_outer() {
        let outer = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events_a \
             GROUP BY event_date, user_id \
             UNION ALL \
             SELECT event_date, user_id, COUNT(*) as cnt FROM events_b \
             GROUP BY user_id",
        );
        assert!(scope_group_by_alignment(&outer, "event_date").is_aligned());

        let branch2 = outer.union_select().expect("second UNION branch");
        assert!(
            !scope_group_by_alignment(&branch2, "event_date").is_aligned(),
            "branch 2's own GROUP BY (user_id only) must not inherit branch 1's alignment"
        );
    }
}
