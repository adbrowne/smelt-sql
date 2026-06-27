//! Pure type inference for pipe SQL (`|>`) queries.
//!
//! This module provides:
//! 1. `infer_pipe_stage_output_schema` — given an input schema and a PIPE_STAGE,
//!    compute the schema of the stage's output.
//! 2. `check_pipe_undeclared_columns` — walk all stages in a pipe query, threading
//!    the running output schema, and report UndeclaredColumn errors for each stage.
//!
//! # Pure-function rule
//!
//! No Salsa imports, no `#[salsa::tracked]`. Callers build inputs via Salsa queries
//! and pass them as plain data.

use smelt_parser::ast::{Expr, PipeQuery, PipeStage, SelectList};
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind};
use smelt_types::{DataType, TypedColumn, UnknownReason};

use super::dispatch::infer_expression_type;
use super::type_context::TypeContext;
use super::UndeclaredColumnInfo;

// ── Schema effect helpers ───────────────────────────────────────────────────

/// Infer the output schema after applying a single PIPE_STAGE to an input schema.
///
/// The `input_schema` is the `(name, TypedColumn)` list from the previous stage.
/// Returns the new `(name, TypedColumn)` list for the next stage.
///
/// Per-operator rules:
/// - **EXTEND**: keep all input columns, then append each `<expr> AS <alias>` item.
/// - **SET**: for each `<col> = <expr>`, replace that column's type in-place.
/// - **DROP**: remove named columns, keep the rest.
/// - **RENAME**: rename `<old> AS <new>` for each pair.
/// - **AS**, **WHERE**, **ORDER BY**, **LIMIT**, **DISTINCT**: pass through unchanged.
/// - **SELECT**: project to the listed expressions (same logic as standard SELECT).
/// - Other operators (AGGREGATE, JOIN, set-ops): pass through unchanged.
pub fn infer_pipe_stage_output_schema(
    input_schema: &[(String, TypedColumn)],
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let op_kind = match stage.op_kind() {
        Some(k) => k,
        None => return input_schema.to_vec(),
    };

    match op_kind {
        PIPE_OP_EXTEND => apply_extend(input_schema, stage, ctx),
        PIPE_OP_SET => apply_set(input_schema, stage, ctx),
        PIPE_OP_DROP => apply_drop(input_schema, stage),
        PIPE_OP_RENAME => apply_rename(input_schema, stage),
        PIPE_OP_SELECT => apply_select(input_schema, stage, ctx),
        PIPE_OP_AGGREGATE => apply_aggregate(input_schema, stage, ctx),
        // AS, WHERE, ORDER BY, LIMIT, DISTINCT — schema passes through unchanged
        PIPE_OP_AS | PIPE_OP_WHERE | PIPE_OP_ORDER_BY | PIPE_OP_LIMIT | PIPE_OP_DISTINCT => {
            input_schema.to_vec()
        }
        // JOIN, set-ops — not in Phase 4 scope; pass through
        _ => input_schema.to_vec(),
    }
}

/// Check undeclared-column diagnostics across all stages of a pipe query.
///
/// For each stage, builds a `TypeContext` with ONLY the previous stage's
/// output columns as model columns (under a dummy qualifier "pipe"), then
/// walks the stage's expressions for undeclared column references.
///
/// Returns a `Vec<UndeclaredColumnInfo>` across all stages.
pub fn check_pipe_undeclared_columns(
    pipe_query: &PipeQuery,
    base_ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    // Extract the FROM-stage schema from the base context.
    // The base_ctx was built by type_context() and contains columns for
    // whatever models/sources appear in the FROM clause.
    // We use the context directly for the first stage and derive subsequent
    // schemas by folding.

    let mut result = Vec::new();

    // Running schema: starts as the columns visible from the FROM clause.
    // We derive this from the base_ctx by looking up what models are registered.
    let mut running_schema: Vec<(String, TypedColumn)> =
        extract_initial_schema_from_ctx(base_ctx, pipe_query);

    for stage in pipe_query.stages() {
        // Build a fresh context with only the running_schema columns.
        let stage_ctx = build_stage_context(&running_schema, base_ctx);

        // Check expressions in this stage for undeclared columns.
        let stage_diags = check_stage_undeclared_columns(&stage, &stage_ctx);
        result.extend(stage_diags);

        // Advance the running schema.
        running_schema = infer_pipe_stage_output_schema(&running_schema, &stage, &stage_ctx);
    }

    result
}

// ── Per-operator schema effect functions ────────────────────────────────────

/// EXTEND: keep all input columns, append each `<expr> AS <alias>` item.
fn apply_extend(
    input: &[(String, TypedColumn)],
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    let mut output = input.to_vec();
    let new_cols = collect_extend_items(stage, ctx);
    output.extend(new_cols);
    output
}

/// SET: for each `<col> = <expr>`, replace that column's type in-place.
fn apply_set(
    input: &[(String, TypedColumn)],
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    let mut output = input.to_vec();
    let assignments = collect_set_assignments(stage, ctx);
    for (col_name, new_typed) in assignments {
        if let Some(entry) = output.iter_mut().find(|(n, _)| n == &col_name) {
            entry.1 = new_typed;
        }
    }
    output
}

/// DROP: remove each named column.
fn apply_drop(input: &[(String, TypedColumn)], stage: &PipeStage) -> Vec<(String, TypedColumn)> {
    let drop_names = collect_drop_names(stage);
    input
        .iter()
        .filter(|(name, _)| !drop_names.contains(name))
        .cloned()
        .collect()
}

/// RENAME: rename each `<old> AS <new>`.
fn apply_rename(input: &[(String, TypedColumn)], stage: &PipeStage) -> Vec<(String, TypedColumn)> {
    let renames = collect_rename_pairs(stage);
    input
        .iter()
        .map(|(name, tc)| {
            if let Some((_, new_name)) = renames.iter().find(|(old, _)| old == name) {
                (new_name.clone(), tc.clone())
            } else {
                (name.clone(), tc.clone())
            }
        })
        .collect()
}

/// SELECT: project to the listed expressions.
fn apply_select(
    _input: &[(String, TypedColumn)],
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    // Find the SELECT_LIST child within the PIPE_STAGE
    use SyntaxKind::SELECT_LIST;
    let select_list_node = stage.syntax().children().find(|c| c.kind() == SELECT_LIST);
    let Some(sl_node) = select_list_node else {
        return Vec::new();
    };
    let Some(sl) = SelectList::cast(sl_node) else {
        return Vec::new();
    };

    let mut output = Vec::new();
    for (i, item) in sl.items().enumerate() {
        let col_name = if let Some(alias) = item.alias() {
            alias
        } else if let Some(expr) = item.expression() {
            infer_column_name_from_expr(&expr).unwrap_or_else(|| format!("col{}", i + 1))
        } else {
            format!("col{}", i + 1)
        };

        let typed_col = if let Some(expr) = item.expression() {
            infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown(UnknownReason::Dynamic),
                nullable: true,
            })
        } else {
            TypedColumn {
                data_type: DataType::Unknown(UnknownReason::Dynamic),
                nullable: true,
            }
        };
        output.push((col_name, typed_col));
    }
    output
}

// ── Stage-level undeclared-column checking ───────────────────────────────────

/// Check undeclared columns for a single PIPE_STAGE's expressions.
///
/// Walks expressions in the stage body and collects any column references
/// that do not resolve in `stage_ctx`.
fn check_stage_undeclared_columns(
    stage: &PipeStage,
    stage_ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    use SyntaxKind::*;

    let op_kind = match stage.op_kind() {
        Some(k) => k,
        None => return Vec::new(),
    };

    let mut result = Vec::new();

    // Collect expressions to check based on operator type.
    let exprs_to_check: Vec<Expr> = match op_kind {
        PIPE_OP_WHERE => collect_where_exprs(stage),
        PIPE_OP_EXTEND => collect_extend_exprs(stage),
        PIPE_OP_SET => collect_set_exprs(stage),
        PIPE_OP_SELECT => collect_select_exprs(stage),
        PIPE_OP_ORDER_BY => collect_order_by_exprs(stage),
        PIPE_OP_AGGREGATE => collect_aggregate_exprs(stage),
        // AS, LIMIT, DISTINCT, DROP, RENAME, JOIN, set-ops — no expression-level column check
        // (DROP and RENAME reference column names at a token level, not expression level,
        // and the existing undeclared-column check handles pure expression refs).
        _ => Vec::new(),
    };

    for expr in exprs_to_check {
        check_expr_for_undeclared(&expr, stage_ctx, &mut result);
    }

    result
}

/// Recursively walk an expression and collect undeclared column references.
fn check_expr_for_undeclared(expr: &Expr, ctx: &TypeContext, out: &mut Vec<UndeclaredColumnInfo>) {
    // Skip subqueries / EXISTS (different scope).
    if expr.as_exists().is_some() || expr.as_subquery().is_some() {
        return;
    }

    // Leaf: column reference.
    let has_expr_children = expr.syntax().children().any(|c| Expr::cast(c).is_some());
    if !has_expr_children {
        if let Some(col_ref) = expr.as_column_ref() {
            let qualifier = col_ref.qualifier();
            let col_name = col_ref.name();
            let lower = col_name.to_lowercase();

            // Skip SQL keywords parsed as identifiers.
            if matches!(lower.as_str(), "true" | "false" | "null") {
                return;
            }

            if ctx.lookup_identifier(qualifier, col_name).is_some() {
                return;
            }

            let message = if let Some(q) = qualifier {
                if let Some(desc) = ctx.describe_qualifier(q) {
                    format!("Column '{}' not found in {}", col_name, desc)
                } else {
                    format!("Column '{}.{}' not found", q, col_name)
                }
            } else {
                format!(
                    "Column '{}' not found in any source, model, or CTE",
                    col_name
                )
            };

            out.push(UndeclaredColumnInfo {
                message,
                range: expr.text_range(),
                qualifier: qualifier.map(|s| s.to_string()),
                column_name: col_name.to_string(),
            });
            return;
        }
    }

    // Recurse into child expressions.
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            check_expr_for_undeclared(&child_expr, ctx, out);
        }
    }
}

// ── Expression collectors ────────────────────────────────────────────────────

/// Collect the WHERE expression from a PIPE_OP_WHERE stage.
fn collect_where_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    stage
        .syntax()
        .children_with_tokens()
        .filter_map(|elem| match elem {
            SyntaxElement::Node(n) if n.kind() != PIPE_OP_WHERE => Expr::cast(n),
            _ => None,
        })
        .collect()
}

/// Collect ORDER BY expressions from a PIPE_OP_ORDER_BY stage.
fn collect_order_by_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    let mut exprs = Vec::new();
    for child in stage.syntax().children() {
        if child.kind() == ORDER_BY_CLAUSE {
            // Walk ORDER_BY_CLAUSE children for EXPRESSION nodes
            for desc in child.children() {
                if let Some(expr) = Expr::cast(desc) {
                    exprs.push(expr);
                }
            }
        }
    }
    exprs
}

/// Collect expression nodes from a PIPE_OP_EXTEND stage.
fn collect_extend_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    stage
        .syntax()
        .children_with_tokens()
        .filter_map(|elem| match elem {
            SyntaxElement::Node(n) if n.kind() != PIPE_OP_EXTEND => Expr::cast(n),
            _ => None,
        })
        .collect()
}

/// Collect the value expressions from a PIPE_OP_SET stage (not the LHS column names).
fn collect_set_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    // The structure is: [PIPE_OP_SET] [IDENT =] [EXPRESSION] [, IDENT = EXPRESSION ...]
    // We collect only EXPRESSION (non-marker) nodes.
    stage
        .syntax()
        .children_with_tokens()
        .filter_map(|elem| match elem {
            SyntaxElement::Node(n) if n.kind() != PIPE_OP_SET => Expr::cast(n),
            _ => None,
        })
        .collect()
}

/// Collect expressions from a PIPE_OP_SELECT stage's SELECT_LIST.
fn collect_select_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    let Some(sl_node) = stage.syntax().children().find(|c| c.kind() == SELECT_LIST) else {
        return Vec::new();
    };
    let Some(sl) = SelectList::cast(sl_node) else {
        return Vec::new();
    };
    sl.items().filter_map(|item| item.expression()).collect()
}

// ── EXTEND item parsing ──────────────────────────────────────────────────────

/// Parse `EXTEND` stage items into `(alias, TypedColumn)` pairs.
///
/// Parser structure for EXTEND stage:
/// `PIPE_STAGE { PIPE_OP_EXTEND  EXPRESSION  [AS_KW  IDENT]  [COMMA  EXPRESSION  ...] }`
fn collect_extend_items(stage: &PipeStage, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let mut items: Vec<(String, TypedColumn)> = Vec::new();

    // Walk children: collect (expression, optional_alias) pairs.
    // State machine: we collect Expression nodes, then look for an alias after each.
    let children: Vec<SyntaxElement> = stage.syntax().children_with_tokens().collect();
    let mut i = 0;

    // Skip past the PIPE_OP_EXTEND marker (first child node).
    while i < children.len() {
        if let SyntaxElement::Node(ref n) = children[i] {
            if n.kind() == PIPE_OP_EXTEND {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    let col_count = items.len(); // will track generated col names
    let mut item_idx = 0usize;

    while i < children.len() {
        // Expect an EXPRESSION node
        let expr_node = match &children[i] {
            SyntaxElement::Node(n) if Expr::cast(n.clone()).is_some() => {
                let expr = Expr::cast(n.clone());
                i += 1;
                expr
            }
            SyntaxElement::Token(t) if t.kind() == COMMA => {
                i += 1;
                continue;
            }
            SyntaxElement::Token(t) if t.kind().is_trivia() => {
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        };

        let Some(expr) = expr_node else {
            continue;
        };

        // Look ahead for alias: optional AS_KW IDENT or bare IDENT
        let mut alias: Option<String> = None;
        let mut j = i;
        while j < children.len() {
            match &children[j] {
                SyntaxElement::Token(t) if t.kind().is_trivia() => j += 1,
                SyntaxElement::Token(t) if t.kind() == AS_KW => {
                    j += 1;
                    // Find the following IDENT
                    while j < children.len() {
                        match &children[j] {
                            SyntaxElement::Token(t) if t.kind().is_trivia() => j += 1,
                            SyntaxElement::Token(t) if t.kind() == IDENT => {
                                alias = Some(t.text().to_string());
                                j += 1;
                                break;
                            }
                            _ => break,
                        }
                    }
                    break;
                }
                SyntaxElement::Token(t) if t.kind() == IDENT => {
                    alias = Some(t.text().to_string());
                    j += 1;
                    break;
                }
                SyntaxElement::Token(t) if t.kind() == COMMA => break,
                _ => break,
            }
        }
        i = j;

        let col_name = alias.unwrap_or_else(|| {
            infer_column_name_from_expr(&expr)
                .unwrap_or_else(|| format!("col{}", col_count + item_idx + 1))
        });

        let typed_col = infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
            data_type: DataType::Unknown(UnknownReason::Dynamic),
            nullable: true,
        });

        items.push((col_name, typed_col));
        item_idx += 1;
    }

    items
}

/// Parse SET assignments: `col = expr, ...` → `Vec<(col_name, TypedColumn)>`.
fn collect_set_assignments(stage: &PipeStage, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let mut result = Vec::new();
    let children: Vec<SyntaxElement> = stage.syntax().children_with_tokens().collect();
    let mut i = 0;

    // Skip PIPE_OP_SET marker
    while i < children.len() {
        if let SyntaxElement::Node(ref n) = children[i] {
            if n.kind() == PIPE_OP_SET {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    while i < children.len() {
        // Skip trivia
        match &children[i] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => {
                i += 1;
                continue;
            }
            SyntaxElement::Token(t) if t.kind() == COMMA => {
                i += 1;
                continue;
            }
            _ => {}
        }

        // Expect: IDENT (column name)
        let col_name = match &children[i] {
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                let name = t.text().to_string();
                i += 1;
                name
            }
            _ => {
                i += 1;
                continue;
            }
        };

        // Skip trivia and EQ
        while i < children.len() {
            match &children[i] {
                SyntaxElement::Token(t) if t.kind().is_trivia() => i += 1,
                SyntaxElement::Token(t) if t.kind() == EQ => {
                    i += 1;
                    break;
                }
                _ => break,
            }
        }

        // Skip trivia
        while i < children.len() {
            if let SyntaxElement::Token(t) = &children[i] {
                if t.kind().is_trivia() {
                    i += 1;
                    continue;
                }
            }
            break;
        }

        // Expect: EXPRESSION
        let typed_col = match &children[i] {
            SyntaxElement::Node(n) => {
                if let Some(expr) = Expr::cast(n.clone()) {
                    i += 1;
                    infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
                        data_type: DataType::Unknown(UnknownReason::Dynamic),
                        nullable: true,
                    })
                } else {
                    i += 1;
                    continue;
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };

        result.push((col_name, typed_col));
    }

    result
}

/// AGGREGATE: replace scope with group keys (in order) then aggregate outputs.
///
/// Output column order per spec: grouping keys first, then aggregate expressions.
fn apply_aggregate(
    _input: &[(String, TypedColumn)],
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    let group_keys = collect_aggregate_group_keys(stage, ctx);
    let agg_items = collect_aggregate_items(stage, ctx);
    let mut output = group_keys;
    output.extend(agg_items);
    output
}

/// Parse group-by keys from an AGGREGATE stage: items after GROUP_KW + BY_KW.
///
/// Returns `(alias, TypedColumn)` pairs.
fn collect_aggregate_group_keys(
    stage: &PipeStage,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let children: Vec<SyntaxElement> = stage.syntax().children_with_tokens().collect();

    // Find the position of GROUP_KW in the children list.
    let group_pos = children
        .iter()
        .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == GROUP_KW));
    let Some(group_pos) = group_pos else {
        // No GROUP BY → no group keys (full-table aggregation).
        return Vec::new();
    };

    // Skip past GROUP_KW and BY_KW.
    let mut i = group_pos + 1;
    while i < children.len() {
        if let SyntaxElement::Token(t) = &children[i] {
            if t.kind() == BY_KW {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    // Parse (expression, optional alias) items from the group-by list.
    collect_expr_alias_items(&children, i, ctx, None)
}

/// Parse aggregate expressions from an AGGREGATE stage: items before GROUP_KW.
fn collect_aggregate_items(stage: &PipeStage, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let children: Vec<SyntaxElement> = stage.syntax().children_with_tokens().collect();

    // Find the position of GROUP_KW.
    let end_pos = children
        .iter()
        .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == GROUP_KW));
    // If no GROUP_KW, all children are aggregate expressions.
    let end_pos = end_pos.unwrap_or(children.len());

    // Skip past PIPE_OP_AGGREGATE marker and AGGREGATE keyword IDENT.
    let mut start = 0;
    // Skip PIPE_OP_AGGREGATE zero-width node.
    while start < end_pos {
        if let SyntaxElement::Node(n) = &children[start] {
            if n.kind() == PIPE_OP_AGGREGATE {
                start += 1;
                break;
            }
        }
        start += 1;
    }
    // Skip the "AGGREGATE" contextual keyword IDENT and trivia.
    while start < end_pos {
        match &children[start] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => start += 1,
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                start += 1;
                break;
            }
            _ => break,
        }
    }

    collect_expr_alias_items(&children, start, ctx, Some(end_pos))
}

/// Collect all expressions from an AGGREGATE stage for undeclared-column checking.
///
/// Returns both the aggregate expressions AND the group-by expressions (all reference
/// input schema columns, not output schema columns).
fn collect_aggregate_exprs(stage: &PipeStage) -> Vec<Expr> {
    use SyntaxKind::*;
    stage
        .syntax()
        .children_with_tokens()
        .filter_map(|elem| match elem {
            SyntaxElement::Node(n) if n.kind() != PIPE_OP_AGGREGATE => Expr::cast(n),
            _ => None,
        })
        .collect()
}

/// Shared helper: parse `(expression [AS alias], ...)` items from a children slice.
///
/// `start`: index to begin scanning.
/// `end`: optional exclusive upper bound (None = scan to end of children).
///
/// Returns `(alias, TypedColumn)` pairs.
fn collect_expr_alias_items(
    children: &[SyntaxElement],
    start: usize,
    ctx: &TypeContext,
    end: Option<usize>,
) -> Vec<(String, TypedColumn)> {
    use SyntaxKind::*;

    let limit = end.unwrap_or(children.len());
    let mut items: Vec<(String, TypedColumn)> = Vec::new();
    let mut i = start;
    let mut item_idx = 0usize;

    while i < limit {
        // Skip trivia and commas.
        match &children[i] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => {
                i += 1;
                continue;
            }
            SyntaxElement::Token(t) if t.kind() == COMMA => {
                i += 1;
                continue;
            }
            _ => {}
        }

        // Expect an EXPRESSION node.
        let expr = match &children[i] {
            SyntaxElement::Node(n) => {
                if let Some(e) = Expr::cast(n.clone()) {
                    i += 1;
                    e
                } else {
                    i += 1;
                    continue;
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };

        // Look ahead for optional alias (AS_KW IDENT or bare IDENT).
        let mut alias: Option<String> = None;
        let mut j = i;
        while j < limit {
            match &children[j] {
                SyntaxElement::Token(t) if t.kind().is_trivia() => j += 1,
                SyntaxElement::Token(t) if t.kind() == AS_KW => {
                    j += 1;
                    while j < limit {
                        match &children[j] {
                            SyntaxElement::Token(t) if t.kind().is_trivia() => j += 1,
                            SyntaxElement::Token(t) if t.kind() == IDENT => {
                                alias = Some(t.text().to_string());
                                j += 1;
                                break;
                            }
                            _ => break,
                        }
                    }
                    break;
                }
                SyntaxElement::Token(t) if t.kind() == IDENT => {
                    alias = Some(t.text().to_string());
                    j += 1;
                    break;
                }
                SyntaxElement::Token(t) if t.kind() == COMMA => break,
                _ => break,
            }
        }
        i = j;

        let col_name = alias.unwrap_or_else(|| {
            infer_column_name_from_expr(&expr).unwrap_or_else(|| format!("col{}", item_idx + 1))
        });

        let typed_col = infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
            data_type: DataType::Unknown(UnknownReason::Dynamic),
            nullable: true,
        });

        items.push((col_name, typed_col));
        item_idx += 1;
    }

    items
}

/// Collect DROP column names from stage: `DROP col1, col2, ...` → `Vec<String>`.
fn collect_drop_names(stage: &PipeStage) -> Vec<String> {
    use SyntaxKind::*;

    let mut names = Vec::new();
    let mut past_marker = false;

    for elem in stage.syntax().children_with_tokens() {
        match elem {
            SyntaxElement::Node(n) if n.kind() == PIPE_OP_DROP => {
                past_marker = true;
            }
            SyntaxElement::Token(t) if past_marker && t.kind() == IDENT => {
                names.push(t.text().to_string());
            }
            _ => {}
        }
    }

    names
}

/// Collect RENAME pairs from stage: `RENAME old AS new, ...` → `Vec<(old, new)>`.
fn collect_rename_pairs(stage: &PipeStage) -> Vec<(String, String)> {
    use SyntaxKind::*;

    let mut pairs = Vec::new();
    let children: Vec<SyntaxElement> = stage.syntax().children_with_tokens().collect();
    let mut i = 0;

    // Skip PIPE_OP_RENAME marker (zero-width node).
    while i < children.len() {
        if let SyntaxElement::Node(ref n) = children[i] {
            if n.kind() == PIPE_OP_RENAME {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    // Skip trivia then the `RENAME` keyword IDENT (it is an IDENT token in the CST
    // because "RENAME" is a contextual keyword, not a reserved-word token).
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => i += 1,
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                i += 1; // consume the "RENAME" keyword
                break;
            }
            _ => break,
        }
    }

    while i < children.len() {
        // Skip trivia and commas
        match &children[i] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => {
                i += 1;
                continue;
            }
            SyntaxElement::Token(t) if t.kind() == COMMA => {
                i += 1;
                continue;
            }
            _ => {}
        }

        // Expect old name IDENT
        let old_name = match &children[i] {
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                let name = t.text().to_string();
                i += 1;
                name
            }
            _ => {
                i += 1;
                continue;
            }
        };

        // Skip trivia, then AS_KW
        while i < children.len() {
            match &children[i] {
                SyntaxElement::Token(t) if t.kind().is_trivia() => i += 1,
                SyntaxElement::Token(t) if t.kind() == AS_KW => {
                    i += 1;
                    break;
                }
                _ => break,
            }
        }

        // Skip trivia, then new name IDENT
        while i < children.len() {
            if let SyntaxElement::Token(t) = &children[i] {
                if t.kind().is_trivia() {
                    i += 1;
                    continue;
                }
            }
            break;
        }

        if i >= children.len() {
            break;
        }

        let new_name = match &children[i] {
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                let name = t.text().to_string();
                i += 1;
                name
            }
            _ => {
                i += 1;
                continue;
            }
        };

        pairs.push((old_name, new_name));
    }

    pairs
}

// ── Context helpers ──────────────────────────────────────────────────────────

/// Build a TypeContext for a stage that contains exactly the given schema columns,
/// plus function signatures from the base context.
///
/// Columns are added under the dummy qualifier `"pipe"` so they are visible as
/// both bare identifiers (via the unqualified lookup path) and `pipe.col` qualified
/// references.
fn build_stage_context(schema: &[(String, TypedColumn)], base_ctx: &TypeContext) -> TypeContext {
    // Clone base_ctx to preserve function signatures, source info, etc.
    // then replace the model columns with the current stage schema.
    let mut ctx = TypeContext::new();

    // Seed function signatures from base context so `infer_expression_type` can
    // resolve smelt.functions.* return types inside pipe stage expressions.
    for (name, sig) in base_ctx.function_signatures_iter() {
        ctx.add_function_signature(name, sig.clone());
    }

    // Add columns under a dummy model name "pipe" so they are accessible
    // as bare identifiers via the unqualified lookup path.
    for (col_name, typed_col) in schema {
        ctx.add_model_column("pipe", col_name, typed_col.clone());
    }

    ctx
}

/// Extract the initial running schema from the base context.
///
/// The base context was built by `type_context()` which populated model columns
/// for the FROM-clause tables. We extract all model columns registered (those
/// belonging to any model qualifier) as the initial schema for stage 0.
///
/// Falls back to an empty schema if no columns are found (e.g. if the table
/// is a reference to a model that has no known schema at analysis time).
fn extract_initial_schema_from_ctx(
    base_ctx: &TypeContext,
    pipe_query: &PipeQuery,
) -> Vec<(String, TypedColumn)> {
    // Try to determine the FROM table name and extract its columns.
    // The FROM clause has a TABLE_REF which may contain a SMELT_PATH_REF or IDENT.
    let from_clause = match pipe_query.from_clause() {
        Some(fc) => fc,
        None => return Vec::new(),
    };

    // Get column names from the context using qualifier resolution.
    // We iterate over all registered model/source/cte columns and find those
    // that belong to the FROM-clause tables.
    //
    // The `TypeContext::columns_for_qualifier` method will return columns for
    // a given qualifier (table name or alias). We need to figure out what
    // qualifier to use.
    let from_qualifiers = extract_from_qualifiers(&from_clause);

    if from_qualifiers.is_empty() {
        // No resolvable qualifiers — return all unqualified model columns
        // as a best-effort schema. This handles simple `FROM t` cases where
        // `t` is registered as a model.
        return base_ctx.all_model_columns_unqualified();
    }

    let mut schema = Vec::new();
    for qualifier in &from_qualifiers {
        let cols = base_ctx.columns_for_qualifier(qualifier);
        for (col_name, typed_col) in cols {
            schema.push((col_name.to_string(), typed_col.clone()));
        }
    }

    schema
}

/// Extract qualifier names from a FROM clause.
///
/// For `FROM smelt.models.t`, returns `["t"]`.
/// For `FROM t`, returns `["t"]`.
/// For `FROM smelt.models.t AS alias`, returns `["alias"]` (alias shadows the base name).
///
/// The returned names are what `TypeContext::columns_for_qualifier` can use to
/// look up columns, including alias resolution.
fn extract_from_qualifiers(from_clause: &smelt_parser::ast::FromClause) -> Vec<String> {
    let mut qualifiers = Vec::new();

    for table_ref in from_clause.table_refs() {
        // Prefer the explicit alias if one is given — the TypeContext registers
        // the alias as a lookup key mapping to the underlying entity name.
        if let Some(alias) = table_ref.alias() {
            qualifiers.push(alias);
            continue;
        }

        // No alias. Check for smelt.models.<name> path ref — use the last segment.
        // The TypeContext's `build_type_context` binds:
        //   alias (or last-segment) → entity_name
        // so looking up the last segment works via alias resolution.
        if let Some(path_ref) = table_ref.smelt_path_ref() {
            if let Some(name) = path_ref.segments().last().cloned() {
                qualifiers.push(name);
                continue;
            }
        }

        // Bare table name (non-smelt reference).
        if let Some(ident) = table_ref.identifier() {
            qualifiers.push(ident);
        }
    }

    qualifiers
}

/// Infer a column name from an expression (bare identifier).
fn infer_column_name_from_expr(expr: &Expr) -> Option<String> {
    if let Some(col_ref) = expr.as_column_ref() {
        return Some(col_ref.name().to_string());
    }
    // For other expression types, return None (will use generated name).
    None
}
