//! Uniform rule → diagnostics interface for built-in planner rules.
//!
//! See `docs/specs/architecture.md` §"Planner scope" (rule → diagnostics
//! interface) and §"Diagnostic parity rule (analysis ↔ build)".
//!
//! Every built-in planner rule surfaces the conditions it rejects as
//! diagnostics through this single interface, evaluated in a sync,
//! side-effect-free `detect` phase. `smelt_db::file_diagnostics` consumes the
//! interface so a rule's verdict reaches the editor and the build identically;
//! the runtime continues to call the rules' classifiers directly, so the
//! build/dispatch verdict is unchanged — it is now *also* visible to the
//! editor. The interface is uniform by design: a future user-authored rule
//! implements the same trait and inherits parity for free.

use std::collections::BTreeSet;

use smelt_core::config::TimeseriesConfig;

use crate::analysis::monotonicity::EventTimeTrace;
use crate::analysis::source_bounds::{derive_model_bounds, BoundContext};
use crate::analysis::{item_alias, select_stmt_items};
use crate::graph::ModelInfo;
use crate::rules::cumulative::{classify_cumulative, KeyedDiagnostic, SourceTimeseriesMap};
use crate::rules::incremental;
use crate::rules::incremental::trace_union_branches;
use crate::types::PartitionGrainConfig;

// ── Parser imports for event-time injectability check ──────────────────────
use smelt_parser::{parse, Cte, File};

/// Severity of a planner-rule diagnostic. `smelt-db` maps this onto its own
/// `DiagnosticSeverity`. Only `Error` blocks the build (Diagnostic parity
/// rule); `Warning` is advisory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSeverity {
    Error,
    Warning,
}

/// Stable identifier for each planner-rule diagnostic. This is the seam the
/// Diagnostic-parity rule consumes: a rule names its rejection here, `smelt-db`
/// maps it to its diagnostic-code catalogue, and the editor and build reach the
/// same verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDiagnosticCode {
    KeyedRequiresGroupBy,
    KeyedUnknownCombiner,
    KeyedGroupByContainsPartitionColumn,
    KeyedForbidsWindowFunctions,
    KeyedForbidsNondeterministic,
    KeyedSnapshotPostureUnsupported,
    KeyedSnapshotSourceUnsupportedColumn,
    KeyedMultipleDrivingSources,
    KeyedSqlNotParseable,
    KeyedOnceWriteUnproven,
    KeyedStateColumnCollision,
    PartitionGrainNotSafe,
    /// An incremental model's `event_time_column` is not accessible at the
    /// outermost SELECT where the time filter is injected — either because the
    /// query is a set operation (UNION/INTERSECT/EXCEPT) or because the FROM
    /// clause is a subquery that does not project the column. Error severity.
    EventTimeColumnNotVisibleAtOuterSelect,
    /// A `grain: partition` model's body calls `smelt.metric(...)`. The
    /// composition of metric expansion with time-filter injection is
    /// deliberately unspecified (`incremental_shapes.md` §"Functions inside
    /// partition-grain bodies"), so the combination refuses ahead of
    /// execution rather than composing unpredictably. Error severity.
    PartitionGrainForbidsMetrics,
}

/// A diagnostic produced by a planner rule's `detect` phase, in rule-native
/// form. The rule does not own source offsets — the consumer anchors the range
/// (per the Diagnostic range encoding rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDiagnostic {
    pub code: RuleDiagnosticCode,
    pub severity: RuleSeverity,
    pub message: String,
}

/// Inputs a built-in rule reads in its sync, side-effect-free `detect` phase.
pub struct RuleContext<'a> {
    /// Model name (for messages).
    pub model_name: &'a str,
    /// Materialization string the build resolves for this model — e.g.
    /// `"cumulative_aggregate"` (the `refresh: keyed` mode's internal scope
    /// key) or `"incremental"`. A rule keys its scope off this exactly as the
    /// build does.
    pub materialization: &'a str,
    /// The model SQL the build will run, with frontmatter stripped (the same
    /// SQL the runtime hands the classifier — see `cumulative.rs`).
    pub sql: &'a str,
    /// `smelt.<path>` refs collected from `sql` (via [`collect_path_refs`]).
    pub refs: &'a [String],
    /// Project-wide `smelt.<path> → timeseries` map for driving-source lookup.
    pub source_timeseries: &'a SourceTimeseriesMap,
    /// Frontmatter `timeseries:` block, if any.
    pub timeseries_config: Option<&'a TimeseriesConfig>,
    /// Frontmatter `incremental:` block, if any.
    pub incremental_config: Option<&'a PartitionGrainConfig>,
    /// The model's own declared `functional_dependencies:` block — the
    /// keyed classifier's once-write family consults it
    /// (`rules::cumulative::classify_once_write`,
    /// `docs/plans/20260809-keyed-frontier.md` Phase 4).
    pub declared_functional_dependencies: &'a [smelt_core::config::FunctionalDependency],
    /// Columns declared `columns.<c>.contract: plausible` in the model's
    /// `.sql` frontmatter — threaded through to `incremental::detect`'s
    /// non-determinism flow/taint check the same way the build itself
    /// resolves it, so the LSP diagnostic and the build agree.
    pub plausible_columns: &'a BTreeSet<String>,
}

/// A planner rule that surfaces its rejections as diagnostics.
///
/// `detect` is sync and side-effect-free. A rule returns an empty vec when the
/// model is outside its scope or clean.
pub trait PlannerRule {
    fn detect(&self, ctx: &RuleContext) -> Vec<RuleDiagnostic>;
}

/// The `refresh: keyed` classifier as a uniform rule.
///
/// Its rejections refuse the model at planning time (`incremental_models.md`
/// §"Key-grain diagnostic codes"), so every one is `Error` — the build hard-refuses on
/// them today via `smelt_planner::classify_cumulative`.
pub struct KeyedRule;

impl PlannerRule for KeyedRule {
    fn detect(&self, ctx: &RuleContext) -> Vec<RuleDiagnostic> {
        if ctx.materialization != "cumulative_aggregate" {
            return Vec::new();
        }
        match classify_cumulative(
            ctx.sql,
            ctx.refs,
            ctx.source_timeseries,
            ctx.timeseries_config.is_some(),
            ctx.declared_functional_dependencies,
        ) {
            Ok(_) => Vec::new(),
            Err(diags) => diags.iter().map(keyed_to_rule).collect(),
        }
    }
}

/// The incremental batch-safety analyzer as a uniform rule.
///
/// Surfaces the incremental safety classifier's rejections as **advisory**
/// (`Warning`) diagnostics. The build does not hard-refuse on these — its
/// dispatch uses `analyze_batch_safety`, which always yields a buildable
/// classification — so they never block the build (Diagnostic parity rule:
/// only `Error` blocks). A missing `timeseries:` block is already surfaced by
/// the frontmatter validator (`TimeseriesRequiredForPartitionGrain`), so this rule
/// stays silent in that case to avoid double-reporting.
pub struct IncrementalRule;

impl PlannerRule for IncrementalRule {
    fn detect(&self, ctx: &RuleContext) -> Vec<RuleDiagnostic> {
        if ctx.materialization != "incremental" {
            return Vec::new();
        }
        let (Some(ts), Some(inc)) = (ctx.timeseries_config, ctx.incremental_config) else {
            return Vec::new();
        };

        // An unspecified composition (metric expansion × time-filter
        // injection) must refuse loudly before any other check runs — no
        // point classifying batch-safety or event-time injectability on a
        // query whose behaviour is undefined.
        if let Some(diag) = check_partition_grain_forbids_metrics(ctx) {
            return vec![diag];
        }

        // Check event-time injectability first. If the event_time_column is not
        // reachable at the outermost SELECT (UNION query or subquery-FROM that
        // doesn't project it), return an Error immediately — no point running
        // batch-safety checks on a query that can't be time-filtered at all.
        if let Some(diag) = check_event_time_injectable(ctx, &ts.event_time_column) {
            return vec![diag];
        }

        // Advisory surfacing of the unified bound map's roll-up
        // (`incremental_shapes.md` §"Batch safety classification"): a
        // `NotDerivable` source is flagged here so the editor sees it, but
        // (per `check_batched_bound_derivable`'s doc comment) the actual
        // fail-closed enforcement with the `--allow-downgrade` escape hatch
        // lives in `smelt_runtime::safety::check_bound_derivation`. This
        // reads the *same* `derive_model_bounds` walk the pushdown-scoping
        // rule and the runtime SQL compiler consume — there is no second,
        // independent derivation deciding the verdict here.
        if let Some(diag) = check_batched_bound_derivable(ctx) {
            return vec![diag];
        }

        let model = ModelInfo {
            name: ctx.model_name.to_string(),
            sql: ctx.sql.to_string(),
            refs: ctx.refs.to_vec(),
            timeseries_config: Some(ts.clone()),
            incremental_config: Some(inc.clone()),
            plausible_columns: ctx.plausible_columns.clone(),
        };
        match incremental::detect(&model) {
            Ok(_) => Vec::new(),
            Err(message) => vec![RuleDiagnostic {
                code: RuleDiagnosticCode::PartitionGrainNotSafe,
                severity: RuleSeverity::Warning,
                message,
            }],
        }
    }
}

/// Returns `Some(Warning)` when any of `ctx`'s declared-timeseries upstream
/// sources has a `NotDerivable` bound — the batch-safety roll-up
/// (`incremental::batch_safety_from_bounds`) cannot classify the model.
/// `None` when every source's bound is derivable (or there are no
/// declared-timeseries sources to bound at all).
///
/// **Advisory, not blocking** — mirrors [`IncrementalRule`]'s existing
/// severity policy (see its doc comment): the actual fail-closed enforcement
/// of `incremental_shapes.md` §"Partition-grain constraints" #10 ("No silent downgrade to
/// full-refresh") happens at the CLI/runtime layer
/// (`smelt_runtime::safety::check_bound_derivation`), which hard-refuses by
/// default and honours the explicit `--allow-downgrade` escape hatch. This
/// diagnostic-parity gate has no notion of that CLI flag — the LSP has no
/// runtime invocation to attach it to — so it stays `Warning` here exactly as
/// `IncrementalRule`'s other checks do, rather than pre-empting
/// `--allow-downgrade` with a harder, unconditional `Error`.
///
/// Builds the same per-ref `BoundContext` the pushdown-scoping walk
/// (`rules::incremental::derive_model_source_bounds`) builds from
/// `ctx.refs`/`ctx.source_timeseries`, so this and the pushdown filter
/// injection can never disagree about which sources are (non-)derivable.
fn check_batched_bound_derivable(ctx: &RuleContext) -> Option<RuleDiagnostic> {
    let mut bound_ctx = BoundContext::new();
    for ref_name in ctx.refs {
        if let Some(ts) = ctx.source_timeseries.get(ref_name) {
            bound_ctx.add_source(ref_name, &ts.partition_column);
        }
    }
    if bound_ctx.source_partition_cols.is_empty() {
        return None;
    }

    let bounds = derive_model_bounds(ctx.sql, &bound_ctx);
    match incremental::batch_safety_from_bounds(&bounds) {
        Ok(_) => None,
        Err(message) => Some(RuleDiagnostic {
            code: RuleDiagnosticCode::PartitionGrainNotSafe,
            severity: RuleSeverity::Warning,
            message: format!("Model '{}': {message}", ctx.model_name),
        }),
    }
}

/// Leaf classifier over the model's own body (per the property composition
/// walk rule): parses `ctx.sql` and walks **every** descendant `FUNCTION_CALL`
/// node — the outer SELECT, any CTE, any derived table — for a namespaced
/// `smelt.metric(...)` call. Returns `Some(Error)` naming the model and the
/// metric argument (if a literal) on the first match; `None` when the body is
/// unparseable (conservative — no other check in this rule can classify it
/// either) or contains no such call.
///
/// `incremental_shapes.md` §"Functions inside partition-grain bodies": the
/// composition of metric expansion with time-filter injection is deliberately
/// unspecified, so the combination refuses outright rather than executing
/// undefined behaviour.
fn check_partition_grain_forbids_metrics(ctx: &RuleContext) -> Option<RuleDiagnostic> {
    let stripped = crate::types::Frontmatter::strip(ctx.sql);
    let parse_result = parse(stripped);
    let root = parse_result.syntax();

    for node in root.descendants() {
        if node.kind() != smelt_parser::SyntaxKind::FUNCTION_CALL {
            continue;
        }
        let Some(func) = smelt_parser::FunctionCall::cast(node) else {
            continue;
        };
        let is_smelt_metric = func
            .namespace()
            .is_some_and(|ns| ns.eq_ignore_ascii_case("smelt"))
            && func
                .name()
                .is_some_and(|name| name.eq_ignore_ascii_case("metric"));
        if !is_smelt_metric {
            continue;
        }
        let arg_text = func
            .arguments()
            .first()
            .map(|arg| arg.syntax().text().to_string());
        let message = match arg_text {
            Some(arg) => format!(
                "Model '{}': `smelt.metric({arg})` is not allowed in a `grain: partition` \
                 model's body — the composition of metric expansion with time-filter injection \
                 is deliberately unspecified (see `incremental_shapes.md` §\"Functions inside \
                 partition-grain bodies\").",
                ctx.model_name
            ),
            None => format!(
                "Model '{}': `smelt.metric(...)` is not allowed in a `grain: partition` \
                 model's body — the composition of metric expansion with time-filter injection \
                 is deliberately unspecified (see `incremental_shapes.md` §\"Functions inside \
                 partition-grain bodies\").",
                ctx.model_name
            ),
        };
        return Some(RuleDiagnostic {
            code: RuleDiagnosticCode::PartitionGrainForbidsMetrics,
            severity: RuleSeverity::Error,
            message,
        });
    }
    None
}

/// Returns `Some(Error)` when the `event_time_column` cannot be injected at
/// the outermost SELECT of `ctx.sql`, `None` when it is safe (or when we
/// cannot determine safety, to be conservative).
///
/// Two cases trigger the error:
/// 1. The query is a set operation (UNION/INTERSECT/EXCEPT) — injecting a
///    WHERE clause at the outer query text only filters the first branch,
///    producing incorrect data, **unless** every branch's projection of
///    `event_time_column` traces back to a real upstream source's own
///    partition column (`Traceable`, per
///    `smelt_logical::analysis::monotonicity::trace_event_time`) — the same
///    per-branch classification the pushdown-scoping walk
///    (`rules::incremental::trace_union_branches`) already performs for
///    batched-model bound derivation, reused here so the two can never
///    reach different verdicts about the same UNION. Only UNION **ALL** is
///    eligible for this relaxation (plain UNION/INTERSECT/EXCEPT keep the
///    unconditional error — their row-combining semantics are not the
///    simple per-branch append `trace_union_branches` assumes). Any branch
///    that isn't `Traceable` (a `StaticSeed` hazard, or genuinely
///    `NotTraceable`) keeps the model rejected.
/// 2. The FROM clause is a bare subquery that does **not** project
///    `event_time_column` — the column is therefore invisible to a WHERE
///    clause added to the outer SELECT.
fn check_event_time_injectable(
    ctx: &RuleContext,
    event_time_column: &str,
) -> Option<RuleDiagnostic> {
    let model_name = ctx.model_name;
    let stripped = crate::types::Frontmatter::strip(ctx.sql);
    let parse_result = parse(stripped);
    let file = File::cast(parse_result.syntax())?;
    let stmt = file.select_stmt()?;

    // Case 1: set operation (UNION/INTERSECT/EXCEPT)
    if stmt.has_set_operation() {
        if stmt.is_union_all() {
            if let Some(diag) =
                check_union_all_injectable(ctx, &stmt, event_time_column, model_name)
            {
                return Some(diag);
            }
            return None;
        }
        return Some(RuleDiagnostic {
            code: RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect,
            severity: RuleSeverity::Error,
            message: format!(
                "Model '{}': `event_time_column` '{}' cannot be injected into a \
                 UNION/INTERSECT/EXCEPT query — a WHERE clause on the outer query \
                 only filters the first branch. Rewrite as a CTE or subquery that \
                 projects '{}' through all branches.",
                model_name, event_time_column, event_time_column
            ),
        });
    }

    // Case 2: bare subquery in FROM that doesn't project event_time_column
    if let Some(from_clause) = stmt.from_clause() {
        let from_text = from_clause.text();
        // Strip the "FROM" keyword and leading whitespace to get the table expression.
        let table_expr = from_text
            .trim()
            .strip_prefix("FROM")
            .or_else(|| from_text.trim().strip_prefix("from"))
            .unwrap_or(&from_text)
            .trim();
        if table_expr.starts_with('(') {
            // Extract the inner SQL from the balanced parentheses.
            if let Some(inner_sql) = extract_balanced_parens(table_expr) {
                if !is_column_projected_in_sql(inner_sql, event_time_column) {
                    return Some(RuleDiagnostic {
                        code: RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect,
                        severity: RuleSeverity::Error,
                        message: format!(
                            "Model '{}': `event_time_column` '{}' is not projected by the \
                             subquery in the FROM clause — it cannot be filtered at the outer \
                             SELECT. Add '{}' to the subquery's SELECT list.",
                            model_name, event_time_column, event_time_column
                        ),
                    });
                }
            }
        } else if let Some(diag) =
            check_cte_from_injectable(&stmt, from_clause, event_time_column, model_name)
        {
            return Some(diag);
        }
    }

    None
}

/// Case 3 of [`check_event_time_injectable`]: the FROM clause names a CTE
/// (rather than a bare parenthesized subquery) that does not project
/// `event_time_column`. Only fires when the outer FROM is a single table
/// expression naming a CTE declared in the statement's own (non-recursive)
/// `WITH` clause — a join, an unresolved name, or a `WITH RECURSIVE` all stay
/// conservative (accepted), since the column may come from elsewhere or the
/// recursive shape can't be safely walked.
fn check_cte_from_injectable(
    stmt: &smelt_parser::SelectStmt,
    from_clause: smelt_parser::FromClause,
    event_time_column: &str,
    model_name: &str,
) -> Option<RuleDiagnostic> {
    let with_clause = stmt.with_clause()?;
    if with_clause.is_recursive() {
        return None; // conservative: recursive CTEs are not walked
    }
    let mut table_refs = from_clause.table_refs();
    let table_ref = table_refs.next()?;
    if table_refs.next().is_some() || from_clause.joins().next().is_some() {
        return None; // conservative: more than one table expression
    }
    let ident = table_ref.identifier()?;
    let cte_map: std::collections::HashMap<String, Cte> = with_clause
        .ctes()
        .filter_map(|c| c.name().map(|n| (n.to_lowercase(), c)))
        .collect();
    if !cte_map.contains_key(&ident.to_lowercase()) {
        return None; // conservative: not a CTE reference (or unresolvable)
    }
    let col_lower = event_time_column.to_lowercase();
    let mut visited = BTreeSet::new();
    if resolve_cte_projects_column(&cte_map, &ident, &col_lower, &mut visited, 0) {
        return None;
    }
    Some(RuleDiagnostic {
        code: RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect,
        severity: RuleSeverity::Error,
        message: format!(
            "Model '{}': `event_time_column` '{}' is not projected by CTE '{}' named in the \
             FROM clause — it cannot be filtered at the outer SELECT. Add '{}' to '{}'s SELECT \
             list.",
            model_name, event_time_column, ident, event_time_column, ident
        ),
    })
}

/// Returns `true` when `event_time_column` (already lower-cased, `col_lower`)
/// is projected by the CTE named `name`, or when the answer can't be proven
/// either way and the conservative (accepting) verdict applies: a wildcard
/// projection, a set-operation body, an unresolvable/cyclic chained
/// reference, or the depth cap. `visited` guards against a cycle turning this
/// into an infinite recursion; the cap is a second, depth-based guard for the
/// same hazard.
fn resolve_cte_projects_column(
    cte_map: &std::collections::HashMap<String, Cte>,
    name: &str,
    col_lower: &str,
    visited: &mut BTreeSet<String>,
    depth: usize,
) -> bool {
    const MAX_CHAIN_DEPTH: usize = 16;
    let name_lower = name.to_lowercase();
    if depth > MAX_CHAIN_DEPTH || !visited.insert(name_lower.clone()) {
        return true; // conservative: depth cap or cycle
    }
    let Some(cte) = cte_map.get(&name_lower) else {
        return true; // conservative: unresolvable name
    };

    let declared = cte.column_names();
    if !declared.is_empty() {
        return declared.iter().any(|c| c.to_lowercase() == col_lower);
    }

    let Some(body) = cte.query().and_then(|q| q.select_stmt()) else {
        return true; // conservative: e.g. a VALUES clause
    };
    if body.has_set_operation() {
        return true; // conservative
    }
    let Some(select_list) = body.select_list() else {
        return true; // conservative
    };
    for item in select_list.items() {
        if item.is_wildcard() {
            return true;
        }
        if let Some(colname) = item.column_name() {
            if colname.to_lowercase() == *col_lower {
                return true;
            }
        }
    }

    // Not directly projected — follow a chained single-table CTE reference.
    if let Some(inner_from) = body.from_clause() {
        let mut inner_refs = inner_from.table_refs();
        if let Some(inner_ref) = inner_refs.next() {
            if inner_refs.next().is_none() && inner_from.joins().next().is_none() {
                if let Some(inner_ident) = inner_ref.identifier() {
                    if cte_map.contains_key(&inner_ident.to_lowercase()) {
                        return resolve_cte_projects_column(
                            cte_map,
                            &inner_ident,
                            col_lower,
                            visited,
                            depth + 1,
                        );
                    }
                }
            }
        }
    }

    false
}

/// Case 1 (UNION ALL) of [`check_event_time_injectable`]: returns
/// `Some(Error)` unless every branch of `stmt` traces its projected
/// `event_time_column` back to a real upstream source's own partition
/// column. Fails closed (returns the same diagnostic as the unconditional
/// pre-B1 check) whenever the outer SELECT doesn't project
/// `event_time_column` at all, or the per-branch trace can't be run —
/// there is nothing to prove injectability from in that case.
fn check_union_all_injectable(
    ctx: &RuleContext,
    stmt: &smelt_parser::SelectStmt,
    event_time_column: &str,
    model_name: &str,
) -> Option<RuleDiagnostic> {
    let reject = || {
        Some(RuleDiagnostic {
            code: RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect,
            severity: RuleSeverity::Error,
            message: format!(
                "Model '{}': `event_time_column` '{}' cannot be injected into a \
                 UNION ALL query — a WHERE clause on the outer query only filters \
                 the first branch, and not every branch's projection of '{}' \
                 traces back to a real upstream source's own partition column. \
                 Rewrite as a CTE or subquery that projects '{}' through all \
                 branches, or ensure each branch directly projects an upstream \
                 timeseries source's partition column.",
                model_name, event_time_column, event_time_column, event_time_column
            ),
        })
    };

    let Some(items) = select_stmt_items(stmt) else {
        return reject();
    };
    let Some(position) = items
        .iter()
        .position(|item| item_alias(item) == event_time_column)
    else {
        return reject();
    };

    // Build the same BoundContext `derive_model_source_bounds` builds from
    // the model's upstream refs — only refs with a declared `timeseries:`
    // participate (lookup sources have no partition column to trace to).
    // Keys are stored *without* the `smelt.` prefix `ctx.refs` carries
    // (`collect_path_refs` keeps it, matching `source_timeseries`'s own
    // keying), because `trace_union_branches`'s per-branch FROM-clause
    // scoping looks sources up by `resolve_table_ref_source_name`'s
    // dot-joined path — which strips the leading `smelt` segment (e.g.
    // `smelt.orders` → `"orders"`). A key mismatch here would silently
    // defeat that scoping and fall back to the unscoped ctx, which can turn
    // a same-named-partition-column UNION branch ambiguous instead of
    // resolving to its own source.
    let mut bound_ctx = BoundContext::new();
    for ref_name in ctx.refs {
        if let Some(ts) = ctx.source_timeseries.get(ref_name) {
            let source_name = ref_name.strip_prefix("smelt.").unwrap_or(ref_name);
            bound_ctx.add_source(source_name, &ts.partition_column);
        }
    }

    let Ok(traces) = trace_union_branches(stmt, event_time_column, position, &bound_ctx) else {
        return reject();
    };

    // A `StaticSeed` branch (a constant/NULL literal in the event-time slot)
    // is named-and-rejected rather than folded into the generic reject
    // message — it is a distinct, positively-proven hazard (not merely
    // "couldn't prove traceable"), so the diagnostic should say so.
    for (i, trace) in traces.iter().enumerate() {
        if let EventTimeTrace::StaticSeed { reason } = trace {
            return Some(RuleDiagnostic {
                code: RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect,
                severity: RuleSeverity::Error,
                message: format!(
                    "Model '{}': UNION ALL branch {} projects a static/NULL seed for \
                     '{}' ({reason}) — not a partitionable stream, so the event-time filter \
                     cannot be proven safe to inject for this UNION ALL.",
                    model_name,
                    i + 1,
                    event_time_column
                ),
            });
        }
    }

    if traces
        .iter()
        .all(|trace| matches!(trace, EventTimeTrace::Traceable { .. }))
    {
        None
    } else {
        reject()
    }
}

/// Returns `true` if `sql` projects `col` (case-insensitive) in its outermost
/// SELECT list — either via a wildcard (`*`) or by an explicit column or alias
/// whose name matches `col`. Returns `true` conservatively when the SQL cannot
/// be parsed.
fn is_column_projected_in_sql(sql: &str, col: &str) -> bool {
    let col_lower = col.to_lowercase();
    let parse_result = parse(sql);
    let Some(file) = File::cast(parse_result.syntax()) else {
        return true; // conservative
    };
    let Some(stmt) = file.select_stmt() else {
        return true; // conservative
    };
    let Some(select_list) = stmt.select_list() else {
        return true; // conservative
    };
    for item in select_list.items() {
        if item.is_wildcard() {
            return true;
        }
        if let Some(name) = item.column_name() {
            if name.to_lowercase() == col_lower {
                return true;
            }
        }
    }
    false
}

/// Extracts the content inside the outermost balanced parentheses from `text`,
/// which must start with `(`. Returns `None` if the parens are unbalanced.
fn extract_balanced_parens(text: &str) -> Option<&str> {
    let mut depth: usize = 0;
    let mut start: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => {
                if depth == 0 {
                    start = Some(i + 1);
                }
                depth += 1;
            }
            ')' => {
                if depth == 0 {
                    return None; // unmatched close
                }
                depth -= 1;
                if depth == 0 {
                    let s = start?;
                    return Some(&text[s..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Run every built-in rule applicable to `ctx` and collect their diagnostics.
///
/// This is the single entry point `smelt_db::file_diagnostics` calls; adding a
/// rule here surfaces it to the editor and the build at once.
pub fn detect_builtin_rules(ctx: &RuleContext) -> Vec<RuleDiagnostic> {
    let mut out = Vec::new();
    out.extend(KeyedRule.detect(ctx));
    out.extend(IncrementalRule.detect(ctx));
    out
}

/// Map a keyed-classifier diagnostic to its uniform rule diagnostic. Every
/// classifier rejection is `Error`.
fn keyed_to_rule(diag: &KeyedDiagnostic) -> RuleDiagnostic {
    let code = match diag {
        KeyedDiagnostic::KeyedRequiresGroupBy => RuleDiagnosticCode::KeyedRequiresGroupBy,
        KeyedDiagnostic::KeyedUnknownCombiner { .. } => RuleDiagnosticCode::KeyedUnknownCombiner,
        KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { .. } => {
            RuleDiagnosticCode::KeyedGroupByContainsPartitionColumn
        }
        KeyedDiagnostic::KeyedForbidsWindowFunctions => {
            RuleDiagnosticCode::KeyedForbidsWindowFunctions
        }
        KeyedDiagnostic::KeyedForbidsNondeterministic { .. } => {
            RuleDiagnosticCode::KeyedForbidsNondeterministic
        }
        KeyedDiagnostic::KeyedSnapshotPostureUnsupported => {
            RuleDiagnosticCode::KeyedSnapshotPostureUnsupported
        }
        KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { .. } => {
            RuleDiagnosticCode::KeyedSnapshotSourceUnsupportedColumn
        }
        KeyedDiagnostic::KeyedMultipleDrivingSources { .. } => {
            RuleDiagnosticCode::KeyedMultipleDrivingSources
        }
        KeyedDiagnostic::KeyedSqlNotParseable => RuleDiagnosticCode::KeyedSqlNotParseable,
        KeyedDiagnostic::KeyedOnceWriteUnproven { .. } => {
            RuleDiagnosticCode::KeyedOnceWriteUnproven
        }
        KeyedDiagnostic::KeyedStateColumnCollision { .. } => {
            RuleDiagnosticCode::KeyedStateColumnCollision
        }
    };
    RuleDiagnostic {
        code,
        severity: RuleSeverity::Error,
        message: diag.to_string(),
    }
}

/// Collect `smelt.<path>` references from raw SQL by scanning for the prefix.
///
/// Returns the deduplicated list of data refs (e.g. `smelt.silver.events`). The
/// single source of ref collection shared by the runtime keyed dispatch
/// and the analysis-layer gate, so both reach the identical driving-source
/// lookup. Conservative: filters out `smelt.functions.*` / `smelt.config.*` /
/// `smelt.define` / `smelt.extern` / `smelt.metric`, which are not data refs.
pub fn collect_path_refs(sql: &str) -> Vec<String> {
    let mut refs = BTreeSet::new();
    let mut chars = sql.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c == 's' && sql[i..].starts_with("smelt.") {
            // Read until a non-identifier character.
            let rest = &sql[i..];
            let end = rest
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '.'))
                .map(|(j, _)| j)
                .unwrap_or(rest.len());
            let candidate = &rest[..end];
            // Filter out function / config references.
            if !candidate.starts_with("smelt.functions.")
                && !candidate.starts_with("smelt.config.")
                && !candidate.starts_with("smelt.define")
                && !candidate.starts_with("smelt.extern")
                && !candidate.starts_with("smelt.metric")
            {
                refs.insert(candidate.to_string());
            }
            // Advance past the consumed identifier.
            for _ in 0..end {
                chars.next();
            }
        } else {
            chars.next();
        }
    }
    refs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::Granularity;
    use std::collections::HashMap;

    fn day_ts() -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }
    }

    fn ts_map() -> SourceTimeseriesMap {
        let mut m = HashMap::new();
        m.insert("smelt.events_ts".to_string(), day_ts());
        m
    }

    #[test]
    fn collect_path_refs_filters_non_data_refs() {
        let sql = "SELECT smelt.functions.f(x), a FROM smelt.events_ts JOIN smelt.silver.x ON true";
        let refs = collect_path_refs(sql);
        assert!(refs.contains(&"smelt.events_ts".to_string()));
        assert!(refs.contains(&"smelt.silver.x".to_string()));
        assert!(!refs.iter().any(|r| r.starts_with("smelt.functions.")));
    }

    #[test]
    fn keyed_rule_flags_unknown_combiner() {
        let sql = "SELECT device_id, STRING_AGG(CAST(amount AS VARCHAR), ',') AS amounts \
                   FROM smelt.events_ts GROUP BY device_id";
        let refs = collect_path_refs(sql);
        let ts = ts_map();
        let ctx = RuleContext {
            model_name: "edges_bad_aggregator",
            materialization: "cumulative_aggregate",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: None,
            incremental_config: None,
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::KeyedUnknownCombiner
                    && d.severity == RuleSeverity::Error),
            "expected KeyedUnknownCombiner Error, got {diags:?}"
        );
    }

    #[test]
    fn keyed_rule_clean_model_is_silent() {
        let sql = "SELECT device_id, user_id, COUNT(*) AS event_count, MIN(amount) AS min_amount \
                   FROM smelt.events_ts WHERE user_id IS NOT NULL GROUP BY device_id, user_id";
        let refs = collect_path_refs(sql);
        let ts = ts_map();
        let ctx = RuleContext {
            model_name: "edges_valid",
            materialization: "cumulative_aggregate",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: None,
            incremental_config: None,
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(
            detect_builtin_rules(&ctx).is_empty(),
            "valid keyed model must produce no rule diagnostics"
        );
    }

    #[test]
    fn non_keyed_materialization_is_silent() {
        let sql = "SELECT 1 AS x";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let ctx = RuleContext {
            model_name: "plain",
            materialization: "table",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: None,
            incremental_config: None,
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(detect_builtin_rules(&ctx).is_empty());
    }

    fn inc_config() -> PartitionGrainConfig {
        PartitionGrainConfig {
            unique_key: vec!["event_date".to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: Default::default(),
        }
    }

    #[test]
    fn incremental_rule_flags_not_batch_safe_as_warning() {
        // Structurally valid incremental model (partition column `event_date`
        // is a SELECT alias and a GROUP BY key), but a LIMIT clause makes it
        // not batch-safe → the incremental safety classifier rejects it.
        // (A group-aligned HAVING here would now be legitimately admitted —
        // `incremental_shapes.md` §"Safety checks" — so LIMIT, which never
        // commutes with the partition filter, is used instead.)
        let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.src \
                   GROUP BY event_date LIMIT 10";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let tsc = day_ts();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "agg_inc",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::PartitionGrainNotSafe
                    && d.severity == RuleSeverity::Warning),
            "expected PartitionGrainNotSafe Warning, got {diags:?}"
        );
    }

    #[test]
    fn incremental_rule_clean_model_is_silent() {
        let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.src GROUP BY event_date";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let tsc = day_ts();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "agg_inc",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(
            detect_builtin_rules(&ctx).is_empty(),
            "a batch-safe incremental model must produce no rule diagnostics; got {:?}",
            detect_builtin_rules(&ctx)
        );
    }

    #[test]
    fn incremental_rule_silent_without_timeseries() {
        // Missing timeseries is already surfaced by the frontmatter validator;
        // the incremental rule must not double-report.
        let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.src \
                   GROUP BY event_date HAVING COUNT(*) > 1";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "agg_inc",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: None,
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(detect_builtin_rules(&ctx).is_empty());
    }

    #[test]
    fn event_time_injectable_into_union_all_when_every_branch_traceable() {
        // UNION ALL model: each branch projects a bare `event_date` column
        // directly from its own declared upstream timeseries source
        // (`smelt.orders`, `smelt.returns`). Per-branch tracing
        // (`trace_union_branches`, shared with pushdown-bound derivation)
        // proves every branch's projection resolves to a real source's own
        // partition column — the simplest `Traceable` case — so the
        // outer-select injectability check no longer rejects this UNION
        // (`model_properties.md` §"Event-time monotonicity trace"). Before B1
        // wired this trace into the diagnostic, *any* set-operation query
        // with a declared `event_time_column` was unconditionally rejected
        // here, regardless of whether its branches were actually traceable.
        let sql = "SELECT event_date, COUNT(*) AS n \
                   FROM smelt.orders GROUP BY event_date \
                   UNION ALL \
                   SELECT event_date, COUNT(*) AS n \
                   FROM smelt.returns GROUP BY event_date";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = inc_config();
        let mut ts_map: SourceTimeseriesMap = HashMap::new();
        ts_map.insert("smelt.orders".to_string(), ts_cfg.clone());
        ts_map.insert("smelt.returns".to_string(), ts_cfg.clone());
        let ctx = RuleContext {
            model_name: "union_mart",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "every UNION ALL branch traces Traceable — must not fire; got: {diags:?}"
        );
    }

    #[test]
    fn event_time_not_injectable_into_union_all_when_a_branch_is_not_traceable() {
        // Same shape as the Traceable case above, but branch 2 projects
        // `MAX(event_ts)` for the partition-column position — a GROUP-BY
        // aggregate, not a per-row monotone image of a source column.
        // `trace_event_time` classifies an unrecognised aggregate function
        // call as `NotTraceable`, so the relaxed check must still reject the
        // UNION (fail closed — not every branch proves traceable).
        let sql = "SELECT event_date, COUNT(*) AS n \
                   FROM smelt.orders GROUP BY event_date \
                   UNION ALL \
                   SELECT MAX(event_ts) AS event_date, COUNT(*) AS n \
                   FROM smelt.returns GROUP BY event_ts";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = inc_config();
        let mut ts_map: SourceTimeseriesMap = HashMap::new();
        ts_map.insert("smelt.orders".to_string(), ts_cfg.clone());
        ts_map.insert(
            "smelt.returns".to_string(),
            smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_ts".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            },
        );
        let ctx = RuleContext {
            model_name: "union_mart",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags.iter().any(|d| d.code
                == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect
                && d.severity == RuleSeverity::Error),
            "UNION ALL with a NotTraceable branch must still produce \
             EventTimeColumnNotVisibleAtOuterSelect Error; got: {diags:?}"
        );
    }

    #[test]
    fn event_time_not_visible_in_subquery_from() {
        // Subquery FROM that doesn't project the event_time_column
        let sql = "SELECT month_start, SUM(amount) AS total \
                   FROM (SELECT DATE_TRUNC('month', event_ts) AS month_start, amount \
                         FROM smelt.orders) sub \
                   GROUP BY 1";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_ts".to_string(), // NOT projected by subquery
            partition_column: "month_start".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = PartitionGrainConfig {
            unique_key: vec!["month_start".to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: Default::default(),
        };
        let ts_map: SourceTimeseriesMap = HashMap::new();
        let ctx = RuleContext {
            model_name: "monthly_mart",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags.iter().any(|d| d.code
                == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect
                && d.severity == RuleSeverity::Error),
            "subquery-FROM model must produce EventTimeColumnNotVisibleAtOuterSelect Error; got: {diags:?}"
        );
    }

    #[test]
    fn event_time_visible_in_single_select_is_clean() {
        // Simple single-SELECT incremental — event_date appears directly
        let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.src GROUP BY event_date";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = inc_config();
        let ts_map: SourceTimeseriesMap = HashMap::new();
        let ctx = RuleContext {
            model_name: "daily_mart",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(
            detect_builtin_rules(&ctx).is_empty(),
            "simple incremental model must be clean; got: {:?}",
            detect_builtin_rules(&ctx)
        );
    }

    #[test]
    fn event_time_not_injectable_into_union_all_static_seed_branch_is_named() {
        // Branch 2 projects a constant literal for the partition-column
        // position — a `StaticSeed` hazard, distinct from a generic
        // `NotTraceable` verdict. The diagnostic message must name it
        // ("static") rather than falling back to the generic UNION-ALL
        // rejection wording, so authors can tell the two failure modes
        // apart.
        let sql = "SELECT event_date, COUNT(*) AS n \
                   FROM smelt.orders GROUP BY event_date \
                   UNION ALL \
                   SELECT DATE '2024-01-01' AS event_date, COUNT(*) AS n \
                   FROM smelt.returns GROUP BY 1";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = inc_config();
        let mut ts_map: SourceTimeseriesMap = HashMap::new();
        ts_map.insert("smelt.orders".to_string(), ts_cfg.clone());
        ts_map.insert("smelt.returns".to_string(), ts_cfg.clone());
        let ctx = RuleContext {
            model_name: "union_static_seed",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        let hit = diags
            .iter()
            .find(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect);
        assert!(
            hit.is_some(),
            "StaticSeed UNION ALL branch must still be rejected; got: {diags:?}"
        );
        assert!(
            hit.unwrap().message.to_lowercase().contains("static"),
            "message must name the StaticSeed hazard, not just the generic rejection: {}",
            hit.unwrap().message
        );
    }

    #[test]
    fn incremental_rule_flags_not_derivable_bound_as_warning() {
        // `bare_lag` derives an `event_date` column but the model's own
        // dependency on `smelt.src` goes through a bare LAG (no explicit
        // RANGE BETWEEN INTERVAL frame) — `derive_model_bounds` cannot prove
        // a bound for `smelt.src`, so the roll-up
        // (`incremental::batch_safety_from_bounds`) refuses and the rule
        // surfaces `PartitionGrainNotSafe` as a Warning here (advisory —
        // `check_batched_bound_derivable`'s doc comment explains why this
        // diagnostic-parity gate stays non-blocking: the fail-closed
        // enforcement with the `--allow-downgrade` escape hatch lives at the
        // CLI/runtime layer, `smelt_runtime::safety::check_bound_derivation`).
        let sql = "SELECT event_date, \
                   LAG(amount) OVER (PARTITION BY device_id ORDER BY event_date) AS prev_amount \
                   FROM smelt.src";
        let refs = collect_path_refs(sql);
        let tsc = day_ts();
        let mut ts_map: SourceTimeseriesMap = HashMap::new();
        ts_map.insert("smelt.src".to_string(), tsc.clone());
        let inc = PartitionGrainConfig {
            unique_key: vec![],
            nondeterministic_columns_retired: (),
            safety_overrides: crate::types::PartitionGrainSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        };
        let ctx = RuleContext {
            model_name: "bare_lag",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::PartitionGrainNotSafe
                    && d.severity == RuleSeverity::Warning),
            "a NotDerivable source bound must surface as an advisory Warning; got: {diags:?}"
        );
    }

    #[test]
    fn check_batched_bound_derivable_silent_when_bound_derivable() {
        // A genuine bounded lookback (Form B: `WHERE col BETWEEN expr -
        // INTERVAL '...' AND expr`) derives cleanly — the bound-derivability
        // gate must stay silent. Exercises `check_batched_bound_derivable`
        // directly (rather than the full `IncrementalRule`) so this test is
        // independent of the separate `OVER`-admissibility check.
        let sql = "SELECT event_date, amount FROM smelt.src \
                   WHERE event_date BETWEEN start_date - INTERVAL '2 days' AND start_date";
        let refs = collect_path_refs(sql);
        let tsc = day_ts();
        let mut ts_map: SourceTimeseriesMap = HashMap::new();
        ts_map.insert("smelt.src".to_string(), tsc.clone());
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "windowed_lookback",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        assert!(
            check_batched_bound_derivable(&ctx).is_none(),
            "a genuinely-derivable bound must not refuse"
        );
    }

    #[test]
    fn event_time_visible_when_subquery_projects_it() {
        // Subquery that DOES project event_ts → EventTimeColumnNotVisibleAtOuterSelect must NOT fire
        let sql = "SELECT event_ts, SUM(amount) AS total \
                   FROM (SELECT event_ts, amount FROM smelt.orders) sub \
                   GROUP BY 1";
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: "event_ts".to_string(),
            partition_column: "event_ts".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = PartitionGrainConfig {
            unique_key: vec!["event_ts".to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: Default::default(),
        };
        let ts_map: SourceTimeseriesMap = HashMap::new();
        let ctx = RuleContext {
            model_name: "proj_mart",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        // Note: PartitionGrainNotSafe Warning may fire (subquery in FROM),
        // but EventTimeColumnNotVisibleAtOuterSelect must NOT fire.
        let diags = detect_builtin_rules(&ctx);
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "must not fire when subquery projects the event_time_column; got: {diags:?}"
        );
    }

    /// Shared harness for the CTE-only outer-visibility cases below: builds a
    /// minimal `RuleContext` around `sql` with `event_time_column` as both the
    /// event-time and partition column, and returns whatever `detect_builtin_rules`
    /// produces.
    fn run_cte_check(sql: &str, event_time_column: &str) -> Vec<RuleDiagnostic> {
        let refs = collect_path_refs(sql);
        let ts_cfg = smelt_core::config::TimeseriesConfig {
            event_time_column: event_time_column.to_string(),
            partition_column: event_time_column.to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        };
        let inc_cfg = PartitionGrainConfig {
            unique_key: vec![event_time_column.to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: Default::default(),
        };
        let ts_map: SourceTimeseriesMap = HashMap::new();
        let ctx = RuleContext {
            model_name: "cte_case",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts_map,
            timeseries_config: Some(&ts_cfg),
            incremental_config: Some(&inc_cfg),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        detect_builtin_rules(&ctx)
    }

    #[test]
    fn cte_not_projecting_event_time_is_rejected() {
        let sql = "WITH recent AS (SELECT user_id, amount FROM smelt.orders) \
                   SELECT user_id, SUM(amount) AS total FROM recent GROUP BY user_id";
        let diags = run_cte_check(sql, "event_ts");
        let hit = diags
            .iter()
            .find(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect);
        assert!(
            hit.is_some(),
            "CTE not projecting event_time_column must be rejected; got: {diags:?}"
        );
        assert!(
            hit.unwrap().message.contains("recent"),
            "message must name the offending CTE: {}",
            hit.unwrap().message
        );
    }

    #[test]
    fn cte_projecting_event_time_is_accepted() {
        let sql = "WITH recent AS (SELECT user_id, event_ts, amount FROM smelt.orders) \
                   SELECT user_id, SUM(amount) AS total FROM recent GROUP BY user_id";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "CTE projecting event_time_column must not be rejected; got: {diags:?}"
        );
    }

    #[test]
    fn cte_wildcard_projection_is_accepted() {
        let sql = "WITH recent AS (SELECT * FROM smelt.orders) \
                   SELECT user_id, SUM(amount) AS total FROM recent GROUP BY user_id";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "wildcard-projecting CTE must be a conservative accept; got: {diags:?}"
        );
    }

    #[test]
    fn chained_cte_missing_event_time_is_rejected() {
        let sql = "WITH a AS (SELECT user_id FROM smelt.orders), \
                        b AS (SELECT user_id FROM a) \
                   SELECT user_id FROM b";
        let diags = run_cte_check(sql, "event_ts");
        let hit = diags
            .iter()
            .find(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect);
        assert!(
            hit.is_some(),
            "chained CTE that never projects event_time_column must be rejected; got: {diags:?}"
        );
        assert!(
            hit.unwrap().message.contains('b'),
            "message must name the CTE the outer FROM binds ('b'), not the root ('a'): {}",
            hit.unwrap().message
        );
    }

    #[test]
    fn cte_column_list_alias_is_used_for_projection() {
        let sql = "WITH recent(user_id, event_ts) AS (SELECT a, b FROM smelt.orders) \
                   SELECT user_id, event_ts FROM recent";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "declared column list, not the body's select list, is the CTE's projection; got: {diags:?}"
        );
    }

    #[test]
    fn multi_table_from_with_cte_is_not_rejected() {
        let sql = "WITH recent AS (SELECT user_id FROM smelt.orders) \
                   SELECT recent.user_id, o.amount FROM recent \
                   JOIN smelt.orders o ON recent.user_id = o.user_id";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "a joined outer FROM must be a conservative accept — the column may come from the \
             other side; got: {diags:?}"
        );
    }

    #[test]
    fn recursive_cte_is_not_rejected() {
        let sql = "WITH RECURSIVE cte AS ( \
                       SELECT user_id, event_ts FROM smelt.orders \
                       UNION ALL \
                       SELECT c.user_id, c.event_ts FROM cte c JOIN smelt.orders o \
                           ON c.user_id = o.user_id \
                   ) SELECT user_id FROM cte";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "a recursive WITH must be a conservative accept, not walked; got: {diags:?}"
        );
    }

    #[test]
    fn plain_table_from_is_unaffected() {
        let sql = "SELECT event_ts, SUM(amount) AS total FROM smelt.orders GROUP BY 1";
        let diags = run_cte_check(sql, "event_ts");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect),
            "a plain table FROM (no CTE) must be unaffected by the CTE case; got: {diags:?}"
        );
    }

    #[test]
    fn partition_grain_metric_call_refuses() {
        let sql = "SELECT event_date, smelt.metric('revenue') AS revenue FROM smelt.orders";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let tsc = day_ts();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "partition_metric",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == RuleDiagnosticCode::PartitionGrainForbidsMetrics)
                .count(),
            1,
            "expected exactly one PartitionGrainForbidsMetrics; got: {diags:?}"
        );
        let hit = diags
            .iter()
            .find(|d| d.code == RuleDiagnosticCode::PartitionGrainForbidsMetrics)
            .unwrap();
        assert_eq!(hit.severity, RuleSeverity::Error);
    }

    #[test]
    fn partition_grain_metric_call_in_cte_refuses() {
        let sql = "WITH derived AS (SELECT event_date, smelt.metric('revenue') AS revenue \
                       FROM smelt.orders) \
                   SELECT event_date, revenue FROM derived";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let tsc = day_ts();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "partition_metric_cte",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::PartitionGrainForbidsMetrics),
            "a smelt.metric() call nested in a CTE must be found by the descendant walk; \
             got: {diags:?}"
        );
    }

    #[test]
    fn partition_grain_without_metric_is_clean() {
        let sql = "SELECT event_date, SUM(amount) AS total, smelt.other_model AS x \
                   FROM smelt.orders GROUP BY event_date";
        let refs = collect_path_refs(sql);
        let ts: SourceTimeseriesMap = HashMap::new();
        let tsc = day_ts();
        let inc = inc_config();
        let ctx = RuleContext {
            model_name: "partition_no_metric",
            materialization: "incremental",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: Some(&tsc),
            incremental_config: Some(&inc),
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::PartitionGrainForbidsMetrics),
            "a partition-grain model with no smelt.metric() call must be clean of this \
             diagnostic; got: {diags:?}"
        );
    }

    #[test]
    fn keyed_model_metric_call_is_unaffected() {
        let sql = "SELECT device_id, smelt.metric('revenue') AS revenue FROM smelt.events_ts \
                   GROUP BY device_id";
        let refs = collect_path_refs(sql);
        let ts = ts_map();
        let ctx = RuleContext {
            model_name: "keyed_metric",
            materialization: "cumulative_aggregate",
            sql,
            refs: &refs,
            source_timeseries: &ts,
            timeseries_config: None,
            incremental_config: None,
            declared_functional_dependencies: &[],
            plausible_columns: &BTreeSet::new(),
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            !diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::PartitionGrainForbidsMetrics),
            "a keyed (non-partition-grain) model must never surface \
             PartitionGrainForbidsMetrics; got: {diags:?}"
        );
    }
}
