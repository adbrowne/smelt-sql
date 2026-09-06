//! Keyed-succession classification — the leaf classifier the composition
//! walk invokes to decide whether a model's SQL is the keyed-succession
//! (SCD2) shape.
//!
//! **Leaf classifier** (`docs/specs/architecture.md` §"Property composition
//! walk rule"): [`classify_keyed_succession`] reasons only over one
//! already-bounded [`SelectNode`]'s own clauses and select list — it does
//! not itself compose through CTEs or set operations. [`super::walk::model_keyed_succession`]
//! is the sole call site, applying the classifier to the model's top scope
//! only and refusing a set operation or unrecognised construct outright
//! (a succession shape nested inside a CTE or `UNION` arm is future work,
//! `docs/specs/incremental_shapes.md` §Future Extensions).
//!
//! See `docs/specs/model_properties.md` §"Keyed-succession classification"
//! for the eleven admission rules this module implements (numbered 1, 1a,
//! 1b, 2–6 in the spec) and the eleven analysis-time diagnostic codes
//! (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md` criterion 2)
//! each refusal reason below maps to 1:1 — [`NotSuccessionReason`] carries
//! the ten reasons that change admission; the eleventh,
//! `SuccessionPreFilterNegatesFlag`, is a `Recognized`-verdict advisory
//! ([`SuccessionAdvisory`]) that never changes admission.

use std::collections::{BTreeSet, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{Expr, SortDirection, SyntaxKind};
use smelt_types::SqlFunction;

use crate::analysis::input_delta::MutationProfile;
use crate::analysis::monotonicity::{trace_event_time, EventTimeTrace, FunctionDeterminism};
use crate::analysis::source_bounds::BoundContext;
use crate::analysis::walk::{InputItem, SelectNode};

/// The driving-source facts [`classify_keyed_succession`] reads — the
/// world-facts a leaf classifier over one already-bounded scope needs but
/// cannot itself derive from that scope's own SQL.
#[derive(Debug, Clone)]
pub struct SuccessionContext {
    /// The name the driving source is referenced by in the model's `FROM`
    /// (e.g. `"raw.customer_changes"`).
    pub source_name: String,
    /// The source's declared `mutation_profile.kind` (`sources.md`). `None`
    /// is the undeclared/unknown case — fails closed exactly like every
    /// other undeclared-profile consumer in this crate ([`super::input_delta`]).
    pub mutation_profile: Option<MutationProfile>,
    /// The source's declared `timeseries.event_time_column`, when it
    /// declares a `timeseries:` block at all. `None` means the source has
    /// no clock — rule 1 refuses regardless of `mutation_profile`.
    pub event_time_column: Option<String>,
    /// Bare (unqualified) names of the source's columns declared `NOT
    /// NULL` — rule 3's key/clock `NOT NULL` proof and rule 6's delete-flag
    /// `NOT NULL` proof both read this set directly rather than re-deriving
    /// it from a full column schema.
    pub not_null_columns: BTreeSet<String>,
}

/// A refusal reason, 1:1 with ten of the eleven analysis-time
/// `Succession*` diagnostic codes (`docs/outcomes/
/// 20260906-scd2-keyed-succession/outcome.md` criterion 2). The eleventh,
/// `SuccessionPreFilterNegatesFlag`, is [`SuccessionAdvisory`] instead — it
/// never refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotSuccessionReason {
    /// `SuccessionWindowFunctionNotLead`: a projected window is not
    /// `LEAD(t)`/`LAG(t)` at the default offset with no default argument.
    WindowFunctionNotLead(String),
    /// `SuccessionPartitionKeyMismatch`: the succession windows do not
    /// share one `PARTITION BY` key set.
    PartitionKeyMismatch(String),
    /// `SuccessionOrderNotMonotoneClock`: the shared `ORDER BY` column is
    /// missing, not ascending-single-key, nullable, or does not trace
    /// `Traceable`-with-`is_strict` to the source's declared
    /// `event_time_column`.
    OrderNotMonotoneClock(String),
    /// `SuccessionIdentityNotProjected`: a key or clock column is not
    /// projected row-locally.
    IdentityNotProjected(String),
    /// `SuccessionRowLocalColumnViolation`: a projected column is not a
    /// row-local function of the current row (an aggregate, a window
    /// sibling, or an unadmitted wrapper operand).
    RowLocalColumnViolation(String),
    /// `SuccessionSingleSourceOnly`: the `FROM` is not exactly one
    /// reference to the declared driving source.
    SingleSourceOnly(String),
    /// `SuccessionDrivingSourceNotAppendOnly`: the driving source is not
    /// declared `append_only` and clocked.
    DrivingSourceNotAppendOnly(String),
    /// `SuccessionPreFilterNotRowLocal`: the pre-window filter is not a
    /// deterministic row-local predicate.
    PreFilterNotRowLocal(String),
    /// `SuccessionDeleteFilterMisplaced`: the post-window filter is not
    /// exactly `QUALIFY NOT <row-local NOT NULL boolean column>`.
    DeleteFilterMisplaced(String),
    /// `SuccessionPatternUnrecognized`: `DISTINCT`/`GROUP BY`/`HAVING`/
    /// `ORDER BY`/`LIMIT` on the scope, or no succession window at all.
    PatternUnrecognized(String),
}

/// `SuccessionPreFilterNegatesFlag` (Warning): a bare negated boolean
/// pre-window filter is admitted, but never closes its predecessor —
/// carried as an advisory on the `Recognized` verdict rather than as a
/// refusal, since it never changes admission (`incremental_shapes.md`
/// §"Delete events").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuccessionAdvisory {
    PreFilterNegatesFlag { column: String },
}

/// The verdict [`classify_keyed_succession`] returns — fail-closed:
/// absence of a proof is [`NotSuccession`](SuccessionVerdict::NotSuccession),
/// never an approximate or partial admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuccessionVerdict {
    Recognized {
        source: String,
        pre_filter: Option<String>,
        key_cols: Vec<String>,
        clock_col: String,
        lead_cols: Vec<String>,
        lag_cols: Vec<String>,
        delete_flag: Option<String>,
        advisories: Vec<SuccessionAdvisory>,
    },
    NotSuccession {
        reason: NotSuccessionReason,
    },
}

/// The leaf classifier: is `node`'s SQL the keyed-succession shape over the
/// driving source described by `ctx`? See the module doc for the rule
/// numbering this implements.
pub fn classify_keyed_succession(node: &SelectNode, ctx: &SuccessionContext) -> SuccessionVerdict {
    use NotSuccessionReason::*;
    let refuse = |reason: NotSuccessionReason| SuccessionVerdict::NotSuccession { reason };

    let select = &node.select;

    // Rule 1b: no other clause on the scope.
    if select.is_distinct() {
        return refuse(PatternUnrecognized("DISTINCT is not this shape".into()));
    }
    if select.group_by_clause().is_some() {
        return refuse(PatternUnrecognized("GROUP BY is not this shape".into()));
    }
    if select.having_clause().is_some() {
        return refuse(PatternUnrecognized("HAVING is not this shape".into()));
    }
    if select.order_by_clause().is_some() {
        return refuse(PatternUnrecognized(
            "ORDER BY on the scope is not this shape — order is a window property, not a scope property"
                .into(),
        ));
    }
    if select.limit_clause().is_some() {
        return refuse(PatternUnrecognized(
            "LIMIT is not this shape — the succession patch cannot maintain a bounded output"
                .into(),
        ));
    }

    // Rule 1: the FROM clause is exactly one reference to the declared
    // driving source.
    if node.inputs.len() != 1 {
        return refuse(SingleSourceOnly(format!(
            "FROM clause has {} inputs — a join is not this shape",
            node.inputs.len()
        )));
    }
    match &node.inputs[0] {
        InputItem::Table { name, .. } if names_match(name, &ctx.source_name) => {}
        InputItem::Table { name, .. } => {
            return refuse(SingleSourceOnly(format!(
                "FROM references '{name}', not the declared driving source '{}'",
                ctx.source_name
            )));
        }
        InputItem::CteRef { .. } => {
            return refuse(SingleSourceOnly(
                "FROM references a CTE, not a single source relation".into(),
            ));
        }
        InputItem::Derived { .. } => {
            return refuse(SingleSourceOnly(
                "FROM is a derived table (subquery), not a single source relation".into(),
            ));
        }
        InputItem::Unsupported { reason } => {
            return refuse(SingleSourceOnly(format!(
                "unrecognised FROM construct: {reason}"
            )));
        }
    }

    // Rule 1 (continued): the driving source must be append-only and
    // clocked.
    let Some(event_time_column) = ctx.event_time_column.clone() else {
        return refuse(DrivingSourceNotAppendOnly(format!(
            "source '{}' declares no timeseries clock",
            ctx.source_name
        )));
    };
    if ctx.mutation_profile != Some(MutationProfile::AppendOnly) {
        return refuse(DrivingSourceNotAppendOnly(format!(
            "source '{}' does not declare mutation_profile.kind: append_only",
            ctx.source_name
        )));
    }

    // Rule 1a: at most one deterministic row-local pre-window filter.
    let mut pre_filter = None;
    let mut advisories = Vec::new();
    if let Some(where_clause) = select.where_clause() {
        let Some(expr) = where_clause.expression() else {
            return refuse(PreFilterNotRowLocal(
                "WHERE clause has no expression".into(),
            ));
        };
        if let Some(negated) = as_bare_not(&expr) {
            if let Some(col) = negated.as_column_ref() {
                advisories.push(SuccessionAdvisory::PreFilterNegatesFlag {
                    column: col.name().to_string(),
                });
                pre_filter = Some(expr.text().trim().to_string());
            } else if is_deterministic_row_local(&expr) {
                pre_filter = Some(expr.text().trim().to_string());
            } else {
                return refuse(PreFilterNotRowLocal(format!(
                    "pre-window filter '{}' is not a deterministic row-local predicate",
                    expr.text().trim()
                )));
            }
        } else if is_deterministic_row_local(&expr) {
            pre_filter = Some(expr.text().trim().to_string());
        } else {
            return refuse(PreFilterNotRowLocal(format!(
                "pre-window filter '{}' is not a deterministic row-local predicate",
                expr.text().trim()
            )));
        }
    }

    // Rules 2 and 5: classify every projected column as a succession
    // window or a row-local plain column.
    let Some(select_list) = select.select_list() else {
        return refuse(PatternUnrecognized("SELECT has no select list".into()));
    };
    let mut window_items: Vec<(String, Expr, WindowCall)> = Vec::new();
    let mut plain_bare_names: HashSet<String> = HashSet::new();
    for item in select_list.items() {
        let alias = item.column_name().unwrap_or_default();
        let Some(expr) = item.expression() else {
            continue;
        };
        let mut windows = find_window_calls(&expr);
        if windows.len() > 1 {
            return refuse(WindowFunctionNotLead(format!(
                "projected column '{alias}' contains more than one window function call"
            )));
        }
        match windows.pop() {
            Some(window_call) => window_items.push((alias, expr, window_call)),
            None => {
                if !is_row_local(&expr) {
                    return refuse(RowLocalColumnViolation(format!(
                        "projected column '{alias}' is not row-local"
                    )));
                }
                if let Some(col) = expr.as_column_ref() {
                    plain_bare_names.insert(col.name().to_string());
                }
            }
        }
    }

    // Rule 2/3: every window is LEAD/LAG(clock) at the default offset,
    // sharing one PARTITION BY key set and one ascending ORDER BY column.
    let mut lead_cols = Vec::new();
    let mut lag_cols = Vec::new();

    let Some(((first_alias, _, first_call), rest)) = window_items.split_first() else {
        return refuse(PatternUnrecognized(
            "no LEAD/LAG window projection found — not a succession shape".into(),
        ));
    };

    let first_shape = match window_shape(first_alias, first_call) {
        Ok(shape) => shape,
        Err(reason) => return refuse(reason),
    };
    let Some(clock_ref) = first_shape.order_expr.as_column_ref() else {
        return refuse(OrderNotMonotoneClock(
            "ORDER BY expression is not a bare column reference".into(),
        ));
    };
    let clock_col = clock_ref.name().to_string();
    let partition_cols = first_shape.partition_cols.clone();
    let shared_order_text = first_shape.order_text.clone();
    let shared_order_expr = first_shape.order_expr.clone();
    if let Err(reason) = record_window(
        first_alias,
        &first_shape,
        &clock_col,
        &mut lead_cols,
        &mut lag_cols,
    ) {
        return refuse(reason);
    }

    for (alias, _item_expr, window_call) in rest {
        let shape = match window_shape(alias, window_call) {
            Ok(shape) => shape,
            Err(reason) => return refuse(reason),
        };
        if shape.partition_cols != partition_cols {
            return refuse(PartitionKeyMismatch(format!(
                "column '{alias}' partitions by a different key set than the other \
                 succession windows"
            )));
        }
        if shape.order_text != shared_order_text {
            return refuse(OrderNotMonotoneClock(format!(
                "column '{alias}' orders by a different column than the other succession \
                 windows"
            )));
        }
        if let Err(reason) = record_window(alias, &shape, &clock_col, &mut lead_cols, &mut lag_cols)
        {
            return refuse(reason);
        }
    }

    // Rule 3 (continued): the clock traces Traceable-with-is_strict to the
    // source's declared event_time_column, and both key and clock are
    // proven NOT NULL.
    let bound_ctx = BoundContext::new().with_source(&ctx.source_name, &event_time_column);
    match trace_event_time(&shared_order_expr, &bound_ctx) {
        EventTimeTrace::Traceable { monotonicity, .. } => {
            if !monotonicity.is_strict {
                return refuse(OrderNotMonotoneClock(format!(
                    "clock column '{clock_col}' traces monotone but not strictly — a colliding \
                     clock cannot maintain the (k, t) identity"
                )));
            }
        }
        EventTimeTrace::StaticSeed { reason } => {
            return refuse(OrderNotMonotoneClock(format!(
                "clock column '{clock_col}' is a constant/static seed: {reason}"
            )));
        }
        EventTimeTrace::NotTraceable { reason, .. } => {
            return refuse(OrderNotMonotoneClock(format!(
                "clock column '{clock_col}' does not trace monotonically to the source's \
                 declared event_time_column '{event_time_column}': {reason}"
            )));
        }
    }

    if !ctx.not_null_columns.contains(&clock_col) {
        return refuse(OrderNotMonotoneClock(format!(
            "clock column '{clock_col}' is nullable"
        )));
    }
    let key_cols: Vec<String> = partition_cols.into_iter().collect();
    for k in &key_cols {
        if !ctx.not_null_columns.contains(k) {
            return refuse(OrderNotMonotoneClock(format!(
                "key column '{k}' is nullable"
            )));
        }
    }

    // Rule 4: key and clock are each projected row-locally.
    for k in key_cols.iter().chain(std::iter::once(&clock_col)) {
        if !plain_bare_names.contains(k) {
            return refuse(IdentityNotProjected(format!(
                "column '{k}' is not projected row-locally"
            )));
        }
    }

    // Rule 2 (continued): a scalar-wrapped window call's other operands
    // are constants, the clock column, or projected row-local columns.
    let allowed_operand_names: HashSet<String> = plain_bare_names
        .iter()
        .cloned()
        .chain(std::iter::once(clock_col.clone()))
        .collect();
    for (alias, item_expr, window_call) in &window_items {
        if item_expr.syntax().text_range() != window_call.enclosing.text_range() {
            if let Err(reason) = validate_wrapper_operands(
                item_expr,
                window_call.func.syntax().text_range(),
                &allowed_operand_names,
            ) {
                return refuse(RowLocalColumnViolation(format!(
                    "scalar wrapper over column '{alias}': {reason}"
                )));
            }
        }
    }

    // Rule 6: at most one post-window filter, exactly `QUALIFY NOT <NOT
    // NULL row-local boolean column>`.
    let mut delete_flag = None;
    if let Some(qualify) = select.qualify_clause() {
        let Some(expr) = qualify.expression() else {
            return refuse(DeleteFilterMisplaced(
                "QUALIFY clause has no expression".into(),
            ));
        };
        let Some(negated) = as_bare_not(&expr) else {
            return refuse(DeleteFilterMisplaced(format!(
                "QUALIFY '{}' is not the exact shape `NOT <column>`",
                expr.text().trim()
            )));
        };
        let Some(col) = negated.as_column_ref() else {
            return refuse(DeleteFilterMisplaced(
                "QUALIFY NOT operand is not a bare column reference".into(),
            ));
        };
        if !ctx.not_null_columns.contains(col.name()) {
            return refuse(DeleteFilterMisplaced(format!(
                "QUALIFY NOT column '{}' is nullable — a NULL flag would drop the row without \
                 recording a tombstone",
                col.name()
            )));
        }
        delete_flag = Some(col.name().to_string());
    }

    SuccessionVerdict::Recognized {
        source: ctx.source_name.clone(),
        pre_filter,
        key_cols,
        clock_col,
        lead_cols,
        lag_cols,
        delete_flag,
        advisories,
    }
}

fn names_match(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `expr` is `NOT <inner>` — the unary boolean negation reuses
/// `BINARY_EXPR`'s node kind with no right operand
/// (`smelt_parser::BinaryExpr::operator`'s own doc comment).
fn as_bare_not(expr: &Expr) -> Option<Expr> {
    let bin = expr.as_binary()?;
    if bin.operator().as_deref() == Some("NOT") && bin.right().is_none() {
        bin.left()
    } else {
        None
    }
}

/// `expr` is a deterministic row-local predicate: no aggregate, window, or
/// subquery ([`is_row_local`]), and no function anywhere in its subtree
/// classifies as run-deterministic or row-nondeterministic under the
/// determinism predicate (`model_properties.md` §"Determinism (run vs row)
/// and the nondeterminism predicate") — the lateness clamp must be stable
/// across runs.
fn is_deterministic_row_local(expr: &Expr) -> bool {
    is_row_local(expr) && !contains_nondeterministic_function(expr)
}

fn contains_nondeterministic_function(expr: &Expr) -> bool {
    expr.syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .filter_map(smelt_parser::FunctionCall::cast)
        .any(|func| {
            let name = func.name().unwrap_or_default();
            !matches!(
                crate::analysis::monotonicity::classify_function_determinism(&name),
                FunctionDeterminism::Neither
            )
        })
}

/// `expr` is a row-local function of the current row alone: no window
/// `OVER`, no subquery, and no aggregate function call anywhere in its
/// subtree.
fn is_row_local(expr: &Expr) -> bool {
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::WINDOW_SPEC)
    {
        return false;
    }
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::SUBQUERY)
    {
        return false;
    }
    !expr
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .filter_map(smelt_parser::FunctionCall::cast)
        .any(|func| {
            let name = func.name().unwrap_or_default().to_uppercase();
            SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate())
        })
}

/// A window-function call found in a select item's expression subtree: the
/// function call and its `OVER` spec, plus the smallest `enclosing` node
/// that carries both as direct children (the wrapping `EXPRESSION` for a
/// bare `LEAD(t) OVER (...) AS x` projection, or the outer `BINARY_EXPR`
/// for a scalar-wrapped one like `LEAD(t) OVER (...) IS NULL AS x` — the
/// parser's checkpoint-based wrapping makes the function call and its
/// `OVER` spec direct siblings of whatever operator wraps them, never
/// nested under their own intermediate `EXPRESSION`).
struct WindowCall {
    func: smelt_parser::FunctionCall,
    window: smelt_parser::WindowSpec,
    enclosing: SyntaxNode,
}

/// Every window-function call (`OVER` present) in `expr`'s own subtree: a
/// node whose direct children include both a `FUNCTION_CALL` and a
/// `WINDOW_SPEC`. Does not recurse into a found call's own children once
/// matched — a window function's argument is never itself another window
/// function in admitted SQL, and rule 2 already refuses a nested window
/// shape via the "more than one window call" arm in
/// [`classify_keyed_succession`] when more than one such node is found
/// across the whole item expression.
fn find_window_calls(expr: &Expr) -> Vec<WindowCall> {
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
struct WindowShape {
    is_lead: bool,
    partition_cols: BTreeSet<String>,
    order_text: String,
    order_expr: Expr,
    arg_col_name: String,
}

fn window_shape(alias: &str, window_call: &WindowCall) -> Result<WindowShape, NotSuccessionReason> {
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
fn record_window(
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
fn validate_wrapper_operands(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::walk::QueryTree;

    fn fixture_ctx() -> SuccessionContext {
        SuccessionContext {
            source_name: "raw.customer_changes".to_string(),
            mutation_profile: Some(MutationProfile::AppendOnly),
            event_time_column: Some("changed_at".to_string()),
            not_null_columns: ["customer_id", "changed_at", "is_deleted"]
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }

    fn classify(sql: &str, ctx: &SuccessionContext) -> SuccessionVerdict {
        let tree = QueryTree::from_sql(sql).expect("sql parses to a query tree");
        let crate::analysis::walk::QueryNode::Select(node) = &tree.root else {
            panic!("expected a top-level SELECT scope, got {:?}", tree.root);
        };
        classify_keyed_succession(node, ctx)
    }

    fn assert_recognized(verdict: &SuccessionVerdict) {
        assert!(
            matches!(verdict, SuccessionVerdict::Recognized { .. }),
            "expected Recognized, got {verdict:?}"
        );
    }

    fn assert_refused_as(verdict: &SuccessionVerdict, expected: fn(&NotSuccessionReason) -> bool) {
        match verdict {
            SuccessionVerdict::NotSuccession { reason } => {
                assert!(expected(reason), "unexpected refusal reason: {reason:?}");
            }
            other => panic!("expected NotSuccession, got {other:?}"),
        }
    }

    // ----- Recognition -----

    #[test]
    fn recognizes_minimal_lead_shape() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        let verdict = classify(sql, &fixture_ctx());
        match &verdict {
            SuccessionVerdict::Recognized {
                lead_cols,
                lag_cols,
                delete_flag,
                pre_filter,
                key_cols,
                clock_col,
                ..
            } => {
                assert_eq!(lead_cols, &["next_ts".to_string()]);
                assert!(lag_cols.is_empty());
                assert_eq!(*delete_flag, None);
                assert_eq!(*pre_filter, None);
                assert_eq!(key_cols, &["customer_id".to_string()]);
                assert_eq!(clock_col, "changed_at");
            }
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn recognizes_lag_projection() {
        let sql = "SELECT customer_id, changed_at, \
                    LAG(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS prev_ts \
                    FROM smelt.raw.customer_changes";
        let verdict = classify(sql, &fixture_ctx());
        match &verdict {
            SuccessionVerdict::Recognized {
                lag_cols,
                lead_cols,
                ..
            } => {
                assert_eq!(lag_cols, &["prev_ts".to_string()]);
                assert!(lead_cols.is_empty());
            }
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn recognizes_scalar_expression_over_lead() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) IS NULL AS is_current \
                    FROM smelt.raw.customer_changes";
        assert_recognized(&classify(sql, &fixture_ctx()));
    }

    #[test]
    fn recognizes_qualify_not_flag_as_delete_flag() {
        let sql = "SELECT customer_id, changed_at, is_deleted, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes QUALIFY NOT is_deleted";
        match classify(sql, &fixture_ctx()) {
            SuccessionVerdict::Recognized { delete_flag, .. } => {
                assert_eq!(delete_flag, Some("is_deleted".to_string()));
            }
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn recognizes_pre_window_clamp_as_pre_filter() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes WHERE changed_at >= DATE '2026-01-01'";
        match classify(sql, &fixture_ctx()) {
            SuccessionVerdict::Recognized { pre_filter, .. } => {
                assert!(pre_filter.is_some());
            }
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn bare_negated_flag_pre_filter_carries_advisory() {
        let with_filter = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes WHERE NOT is_deleted";
        let without_filter = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        let with_verdict = classify(with_filter, &fixture_ctx());
        let without_verdict = classify(without_filter, &fixture_ctx());
        match (&with_verdict, &without_verdict) {
            (
                SuccessionVerdict::Recognized {
                    advisories,
                    pre_filter,
                    key_cols: k1,
                    clock_col: c1,
                    lead_cols: l1,
                    lag_cols: g1,
                    delete_flag: d1,
                    ..
                },
                SuccessionVerdict::Recognized {
                    advisories: advisories2,
                    key_cols: k2,
                    clock_col: c2,
                    lead_cols: l2,
                    lag_cols: g2,
                    delete_flag: d2,
                    ..
                },
            ) => {
                assert_eq!(
                    advisories,
                    &vec![SuccessionAdvisory::PreFilterNegatesFlag {
                        column: "is_deleted".to_string()
                    }]
                );
                assert!(pre_filter.is_some());
                assert!(advisories2.is_empty());
                assert_eq!((k1, c1, l1, g1, d1), (k2, c2, l2, g2, d2));
            }
            other => panic!("expected both Recognized, got {other:?}"),
        }
    }

    // ----- Refusals -----

    #[test]
    fn refuses_non_succession_window_function() {
        let sql = "SELECT customer_id, changed_at, \
                    SUM(customer_id) OVER (PARTITION BY customer_id ORDER BY changed_at) AS total \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_lead_over_other_column() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(customer_id) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_id \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_lead_with_explicit_offset() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at, 2) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_lead_with_default_argument() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at, 1, changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_mixed_partition_keys() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts, \
                    LAG(changed_at) OVER (PARTITION BY changed_at ORDER BY changed_at) AS prev_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PartitionKeyMismatch(_))
        });
    }

    #[test]
    fn refuses_nullable_key() {
        let mut ctx = fixture_ctx();
        ctx.not_null_columns.remove("customer_id");
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_nullable_clock() {
        let mut ctx = fixture_ctx();
        ctx.not_null_columns.remove("changed_at");
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_non_strict_clock() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY CAST(changed_at AS DATE)) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_clock_not_event_time_column() {
        let mut ctx = fixture_ctx();
        ctx.not_null_columns.insert("created_at".to_string());
        let sql = "SELECT customer_id, created_at, \
                    LEAD(created_at) OVER (PARTITION BY customer_id ORDER BY created_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_descending_order() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at DESC) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_second_sort_key() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at, customer_id) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_order_by_expression_not_bare_column() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at + 1) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
        });
    }

    #[test]
    fn refuses_two_window_calls_in_one_projection() {
        let sql = "SELECT customer_id, changed_at, \
                    COALESCE(LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at), \
                    LAG(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at)) AS both \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_unprojected_key() {
        let sql = "SELECT changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::IdentityNotProjected(_))
        });
    }

    #[test]
    fn refuses_unprojected_clock() {
        let sql = "SELECT customer_id, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::IdentityNotProjected(_))
        });
    }

    #[test]
    fn refuses_aggregate_sibling_projection() {
        let sql = "SELECT customer_id, changed_at, COUNT(*) OVER () AS total, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
        });
    }

    #[test]
    fn refuses_non_row_local_projected_column() {
        let sql = "SELECT customer_id, changed_at, (SELECT MAX(x) FROM other) AS bad, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::RowLocalColumnViolation(_))
        });
    }

    #[test]
    fn refuses_join_from() {
        let sql = "SELECT c.customer_id, c.changed_at, \
                    LEAD(c.changed_at) OVER (PARTITION BY c.customer_id ORDER BY c.changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes c JOIN smelt.raw.other o ON c.customer_id = o.customer_id";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::SingleSourceOnly(_))
        });
    }

    #[test]
    fn refuses_cte_from() {
        let sql = "WITH c AS (SELECT * FROM smelt.raw.customer_changes) \
                    SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM c";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::SingleSourceOnly(_))
        });
    }

    #[test]
    fn refuses_subquery_from() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM (SELECT * FROM smelt.raw.customer_changes) t";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::SingleSourceOnly(_))
        });
    }

    #[test]
    fn refuses_mutable_source() {
        let mut ctx = fixture_ctx();
        ctx.mutation_profile = Some(MutationProfile::Mutable);
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
        });
    }

    #[test]
    fn refuses_change_feed_source() {
        let mut ctx = fixture_ctx();
        ctx.mutation_profile = Some(MutationProfile::ChangeFeed);
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
        });
    }

    #[test]
    fn refuses_undeclared_mutation_profile() {
        let mut ctx = fixture_ctx();
        ctx.mutation_profile = None;
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
        });
    }

    #[test]
    fn refuses_unclocked_source() {
        let mut ctx = fixture_ctx();
        ctx.event_time_column = None;
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &ctx), |r| {
            matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
        });
    }

    #[test]
    fn refuses_non_row_local_pre_filter() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes WHERE changed_at >= (SELECT MIN(x) FROM other)";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PreFilterNotRowLocal(_))
        });
    }

    #[test]
    fn refuses_nondeterministic_pre_filter() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes WHERE changed_at <= NOW()";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PreFilterNotRowLocal(_))
        });
    }

    #[test]
    fn refuses_qualify_other_shape() {
        let sql = "SELECT customer_id, changed_at, is_deleted, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes QUALIFY is_deleted";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::DeleteFilterMisplaced(_))
        });
    }

    #[test]
    fn refuses_qualify_nullable_flag() {
        let sql = "SELECT customer_id, changed_at, is_active, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes QUALIFY NOT is_active";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::DeleteFilterMisplaced(_))
        });
    }

    #[test]
    fn refuses_distinct() {
        let sql = "SELECT DISTINCT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PatternUnrecognized(_))
        });
    }

    #[test]
    fn refuses_group_by() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes GROUP BY customer_id, changed_at";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PatternUnrecognized(_))
        });
    }

    #[test]
    fn refuses_order_by() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes ORDER BY changed_at";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PatternUnrecognized(_))
        });
    }

    #[test]
    fn refuses_limit() {
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes LIMIT 10";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PatternUnrecognized(_))
        });
    }

    #[test]
    fn refuses_having() {
        // HAVING requires GROUP BY in real SQL, but the classifier's rule 1b
        // checks the clause's mere presence — refuse before any GROUP BY
        // check would even matter.
        let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes GROUP BY customer_id, changed_at HAVING COUNT(*) > 1";
        assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
            matches!(r, NotSuccessionReason::PatternUnrecognized(_))
        });
    }
}
