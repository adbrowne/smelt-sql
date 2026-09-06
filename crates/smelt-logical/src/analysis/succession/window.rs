//! Window-call discovery and per-window shape checks (rules 2/3) used by
//! [`super::classify_keyed_succession`].

use std::collections::{BTreeSet, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{Expr, SortDirection, SyntaxKind};
use smelt_types::SqlFunction;

use super::NotSuccessionReason;

/// A window-function call found in a select item's expression subtree: the
/// function call and its `OVER` spec, plus the smallest `enclosing` node
/// that carries both as direct children (the wrapping `EXPRESSION` for a
/// bare `LEAD(t) OVER (...) AS x` projection, or the outer `BINARY_EXPR`
/// for a scalar-wrapped one like `LEAD(t) OVER (...) IS NULL AS x` — the
/// parser's checkpoint-based wrapping makes the function call and its
/// `OVER` spec direct siblings of whatever operator wraps them, never
/// nested under their own intermediate `EXPRESSION`).
pub(super) struct WindowCall {
    pub(super) func: smelt_parser::FunctionCall,
    pub(super) window: smelt_parser::WindowSpec,
    pub(super) enclosing: SyntaxNode,
}

/// Every window-function call (`OVER` present) in `expr`'s own subtree: a
/// node whose direct children include both a `FUNCTION_CALL` and a
/// `WINDOW_SPEC`. Does not recurse into a found call's own children once
/// matched — a window function's argument is never itself another window
/// function in admitted SQL, and rule 2 already refuses a nested window
/// shape via the "more than one window call" arm in
/// [`super::classify_keyed_succession`] when more than one such node is
/// found across the whole item expression.
pub(super) fn find_window_calls(expr: &Expr) -> Vec<WindowCall> {
    let mut out = Vec::new();
    find_window_calls_rec(expr.syntax(), &mut out);
    out
}

fn find_window_calls_rec(node: &SyntaxNode, out: &mut Vec<WindowCall>) {
    let mut func_child = None;
    let mut window_child = None;
    for child in node.children() {
        match child.kind() {
            SyntaxKind::FUNCTION_CALL => func_child = Some(child),
            SyntaxKind::WINDOW_SPEC => window_child = Some(child),
            _ => {}
        }
    }
    if let (Some(func_node), Some(window_node)) = (func_child, window_child) {
        if let (Some(func), Some(window)) = (
            smelt_parser::FunctionCall::cast(func_node),
            smelt_parser::WindowSpec::cast(window_node),
        ) {
            out.push(WindowCall {
                func,
                window,
                enclosing: node.clone(),
            });
            return;
        }
    }
    for child in node.children() {
        find_window_calls_rec(&child, out);
    }
}

/// One window call's own shape (rule 2/3's per-item checks), before any
/// comparison against sibling window items.
pub(super) struct WindowShape {
    pub(super) is_lead: bool,
    pub(super) partition_cols: BTreeSet<String>,
    pub(super) order_text: String,
    pub(super) order_expr: Expr,
    pub(super) arg_col_name: String,
}

pub(super) fn window_shape(
    alias: &str,
    window_call: &WindowCall,
) -> Result<WindowShape, NotSuccessionReason> {
    use NotSuccessionReason::*;
    let func = &window_call.func;
    let window = &window_call.window;
    let name = func.name().unwrap_or_default().to_uppercase();
    let is_lead = match name.as_str() {
        "LEAD" => true,
        "LAG" => false,
        other => {
            return Err(WindowFunctionNotLead(format!(
                "'{other}' (column '{alias}') is not LEAD/LAG"
            )));
        }
    };
    let args = func.arguments();
    if args.len() != 1 {
        return Err(WindowFunctionNotLead(format!(
            "{name}(...) for column '{alias}' carries an explicit offset/default argument \
             — only the default single-argument form is admitted"
        )));
    }
    let Some(arg_col) = args[0].as_column_ref() else {
        return Err(WindowFunctionNotLead(format!(
            "{name} argument for column '{alias}' is not a bare column reference"
        )));
    };

    let partition_cols: BTreeSet<String> = match window.partition_by() {
        Some(pb) => {
            let mut set = BTreeSet::new();
            for e in pb.expressions() {
                match e.as_column_ref() {
                    Some(c) => {
                        set.insert(c.name().to_string());
                    }
                    None => {
                        return Err(PartitionKeyMismatch(format!(
                            "PARTITION BY for column '{alias}' contains a non-column expression"
                        )));
                    }
                }
            }
            set
        }
        None => {
            return Err(PartitionKeyMismatch(format!(
                "column '{alias}' window has no PARTITION BY"
            )));
        }
    };

    let Some(order_by) = window.order_by() else {
        return Err(OrderNotMonotoneClock(format!(
            "column '{alias}' window has no ORDER BY"
        )));
    };
    let order_items: Vec<_> = order_by.items().collect();
    if order_items.len() != 1 {
        return Err(OrderNotMonotoneClock(format!(
            "column '{alias}' window ORDER BY carries more than one sort key"
        )));
    }
    if order_items[0].direction() == Some(SortDirection::Desc) {
        return Err(OrderNotMonotoneClock(format!(
            "column '{alias}' window orders descending"
        )));
    }
    let Some(order_expr) = order_items[0].expression() else {
        return Err(OrderNotMonotoneClock(format!(
            "column '{alias}' window ORDER BY has no expression"
        )));
    };
    let order_text = order_expr.text().trim().to_string();

    Ok(WindowShape {
        is_lead,
        partition_cols,
        order_text,
        order_expr,
        arg_col_name: arg_col.name().to_string(),
    })
}

/// Check `shape` reaches over `clock_col` and, if so, record its alias as a
/// lead or lag column.
pub(super) fn record_window(
    alias: &str,
    shape: &WindowShape,
    clock_col: &str,
    lead_cols: &mut Vec<String>,
    lag_cols: &mut Vec<String>,
) -> Result<(), NotSuccessionReason> {
    if shape.arg_col_name != clock_col {
        let name = if shape.is_lead { "LEAD" } else { "LAG" };
        return Err(NotSuccessionReason::WindowFunctionNotLead(format!(
            "{name}(...) for column '{alias}' does not reach over the clock column"
        )));
    }
    if shape.is_lead {
        lead_cols.push(alias.to_string());
    } else {
        lag_cols.push(alias.to_string());
    }
    Ok(())
}

/// Validate a scalar expression wrapping exactly one succession window
/// call (rule 2's "scalar expression over exactly one such call"): every
/// operand outside the window call's own subtree must be a constant, the
/// clock column, or a column also projected row-locally elsewhere in this
/// scope (`allowed_names`). `window_range` identifies the window call's own
/// node so its subtree is treated as opaque (already proven by the caller).
pub(super) fn validate_wrapper_operands(
    expr: &Expr,
    window_range: smelt_parser::TextRange,
    allowed_names: &HashSet<String>,
) -> Result<(), String> {
    if expr.syntax().text_range() == window_range {
        return Ok(());
    }
    if let Some(col) = expr.as_column_ref() {
        return if allowed_names.contains(col.name()) {
            Ok(())
        } else {
            Err(format!(
                "operand '{}' is neither a constant, the clock column, nor a row-local \
                 projected column",
                col.name()
            ))
        };
    }
    if let Some(func) = expr.as_function_call() {
        let name = func.name().unwrap_or_default().to_uppercase();
        if SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate() || f.is_window()) {
            return Err(format!("'{name}' is an aggregate/window function"));
        }
        for arg in func.arguments() {
            validate_wrapper_operands(&arg, window_range, allowed_names)?;
        }
        return Ok(());
    }
    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            validate_wrapper_operands(&side, window_range, allowed_names)?;
        }
        return Ok(());
    }
    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            validate_wrapper_operands(&value, window_range, allowed_names)?;
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                validate_wrapper_operands(&arm, window_range, allowed_names)?;
            }
        }
        if let Some(else_expr) = case.else_expr() {
            validate_wrapper_operands(&else_expr, window_range, allowed_names)?;
        }
        return Ok(());
    }
    if let Some(cast) = expr.as_cast() {
        return match cast.expression() {
            Some(inner) => validate_wrapper_operands(&inner, window_range, allowed_names),
            None => Err("CAST has no inner expression to resolve".to_string()),
        };
    }

    let has_ident = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::IDENT);
    if !has_ident {
        return Ok(());
    }

    Err(format!(
        "expression shape is not recognised: {}",
        expr.text().trim()
    ))
}
