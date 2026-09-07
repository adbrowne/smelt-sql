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

mod predicates;
#[cfg(test)]
mod tests;
mod window;

use std::collections::{BTreeSet, HashSet};

use crate::analysis::input_delta::MutationProfile;
use crate::analysis::monotonicity::{trace_event_time, EventTimeTrace};
use crate::analysis::source_bounds::BoundContext;
use crate::analysis::walk::{InputItem, SelectNode};
use predicates::{as_bare_not, is_deterministic_row_local, names_match};
use window::{
    derived_template, find_window_calls, record_window, validate_wrapper_operands, window_shape,
};

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
        /// Every row-local (non-window) projected column as `(alias, source
        /// expression text)`, in the model's own column order — the
        /// classifier's own expression material, carried forward so
        /// `maintenance::succession::SuccessionRecipe::from_verdict` (and,
        /// through it, the emitters in `maintenance::emit::succession`)
        /// never re-derives it by re-parsing the model's SQL
        /// (`CLAUDE.md` §"Maintenance-plan purity").
        /// Boxed for the same reason as `lead_derived` below.
        row_local: Box<Vec<(String, String)>>,
        /// One entry per `lead_cols` alias: `(alias, expr_template)`, where
        /// `expr_template` is the select item's own expression text with the
        /// `LEAD(...)  OVER (...)` call's span replaced by the literal token
        /// `{lead}` — e.g. `("valid_to", "{lead}")` for a bare projection or
        /// `("is_current", "{lead} IS NULL")` for a scalar-wrapped one. Feeds
        /// [`crate::maintenance::emit::DerivedColumn`] directly.
        /// Boxed per `clippy::large_enum_variant` — `NotSuccession` carries
        /// only a reason string, so leaving this Vec inline would roughly
        /// double every `NotSuccession` allocation-free match arm's stack
        /// footprint to accommodate a payload it never uses.
        lead_derived: Box<Vec<(String, String)>>,
        /// The `LAG` counterpart of `lead_derived`, using the `{lag}` token.
        /// Boxed for the same reason.
        lag_derived: Box<Vec<(String, String)>>,
        /// The `QUALIFY NOT <flag>` operand's own expression text (today
        /// always a bare column name, since rule 6 admits no other shape),
        /// or `None` when the model has no `QUALIFY` clause. Feeds the
        /// emitters' `delete_flag_expr` argument directly. Boxed for the
        /// same reason.
        delete_flag_expr: Box<Option<String>>,
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
    let mut window_items: Vec<(String, smelt_parser::Expr, window::WindowCall)> = Vec::new();
    let mut plain_bare_names: HashSet<String> = HashSet::new();
    let mut row_local: Vec<(String, String)> = Vec::new();
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
                if !predicates::is_row_local(&expr) {
                    return refuse(RowLocalColumnViolation(format!(
                        "projected column '{alias}' is not row-local"
                    )));
                }
                if let Some(col) = expr.as_column_ref() {
                    plain_bare_names.insert(col.name().to_string());
                }
                row_local.push((alias.clone(), expr.text().trim().to_string()));
            }
        }
    }

    // Rule 2/3: every window is LEAD/LAG(clock) at the default offset,
    // sharing one PARTITION BY key set and one ascending ORDER BY column.
    let mut lead_cols = Vec::new();
    let mut lag_cols = Vec::new();
    let mut lead_derived: Vec<(String, String)> = Vec::new();
    let mut lag_derived: Vec<(String, String)> = Vec::new();

    let Some(((first_alias, first_expr, first_call), rest)) = window_items.split_first() else {
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
    let template = derived_template(first_expr, first_call);
    if first_shape.is_lead {
        lead_derived.push((first_alias.clone(), template));
    } else {
        lag_derived.push((first_alias.clone(), template));
    }

    for (alias, item_expr, window_call) in rest {
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
        let template = derived_template(item_expr, window_call);
        if shape.is_lead {
            lead_derived.push((alias.clone(), template));
        } else {
            lag_derived.push((alias.clone(), template));
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
    let mut delete_flag_expr = None;
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
        delete_flag_expr = Some(negated.text().trim().to_string());
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
        row_local: Box::new(row_local),
        lead_derived: Box::new(lead_derived),
        lag_derived: Box::new(lag_derived),
        delete_flag_expr: Box::new(delete_flag_expr),
    }
}
