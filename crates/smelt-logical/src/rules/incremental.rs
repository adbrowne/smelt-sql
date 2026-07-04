use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tracing::info;

use crate::analysis::monotonicity::{
    classify_function_determinism, trace_event_time, EventTimeTrace, FunctionDeterminism,
    NONDETERMINISTIC_FUNCTIONS,
};
use crate::analysis::source_bounds::{
    from_clause_alias_sources, resolve_join_driving_fact, BoundContext, BoundResult, InjectionPoint,
};
use crate::analysis::temporal::{analyze_temporal_dependencies, TemporalOffset};
use crate::analysis::{
    analyze_select, find_item_expr_by_alias_or_position, item_alias, item_expr,
    scope_distinct_alignment, scope_group_by_alignment, select_stmt_items, PartitionAlignment,
    SelectItemKind,
};
use crate::graph::{ModelGraph, ModelInfo};
use crate::types::{Opportunity, OpportunityData, Transformation};

/// How safely a model can be backfilled in large batches.
///
/// Derived from temporal dependency analysis (Phase 3). Models with no
/// cross-partition dependencies can process any range in a single query;
/// models with bounded lookback need chunked execution; models with
/// unbounded dependencies must go per-partition or full refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum BatchSafety {
    /// Single query for any range — aggregations are partition-local.
    FullyBatchSafe,
    /// Safe for bounded chunks — window functions or joins need context rows
    /// but the dependency is bounded.
    BoundedSafe {
        /// Maximum recommended chunk size in days. Each chunk query fetches
        /// `[start - context_days, end)` but only writes `[start, end)`.
        max_chunk_days: u32,
        /// Extra context days needed before each chunk's start.
        context_days: u32,
        /// Human-readable explanation of why chunking is needed.
        reason: String,
    },
    /// Must process per-partition — cross-partition dependencies are unbounded.
    PerPartitionOnly {
        /// Human-readable explanation of why per-partition is required.
        reason: String,
    },
}

/// Analyze how safely a model can be backfilled in batches.
///
/// Uses the SQL's temporal dependency (lookback/lookahead) to determine
/// whether a single query can cover any range, or whether the range must
/// be split into chunks or individual partitions.
pub fn analyze_batch_safety(model: &ModelInfo) -> BatchSafety {
    let sql = crate::types::Frontmatter::strip(&model.sql);
    let temporal = analyze_temporal_dependencies(sql);

    // Determine granularity period for converting Periods→Days
    let period_days = model
        .timeseries_config
        .as_ref()
        .map(|c| crate::analysis::temporal::granularity_period_days(&c.granularity))
        .unwrap_or(1);

    // Check lookback
    let lookback_days = match &temporal.lookback {
        TemporalOffset::Zero => 0,
        TemporalOffset::Periods(n) => n * period_days,
        TemporalOffset::Days(n) => *n,
        TemporalOffset::Unbounded { reason } => {
            return BatchSafety::PerPartitionOnly {
                reason: format!("unbounded lookback: {}", reason),
            };
        }
    };

    // Check lookahead
    let lookahead_days = match &temporal.lookahead {
        TemporalOffset::Zero => 0,
        TemporalOffset::Periods(n) => n * period_days,
        TemporalOffset::Days(n) => *n,
        TemporalOffset::Unbounded { reason } => {
            return BatchSafety::PerPartitionOnly {
                reason: format!("unbounded lookahead: {}", reason),
            };
        }
    };

    let context_days = lookback_days.max(lookahead_days);

    if context_days == 0 {
        BatchSafety::FullyBatchSafe
    } else {
        // Recommend chunks at least 3x the context to keep overhead reasonable.
        // Minimum chunk = 7 days, maximum recommended = 90 days.
        let min_chunk = context_days * 3;
        let max_chunk_days = min_chunk.clamp(7, 90);

        if min_chunk > 90 {
            info!(
                "ideal chunk size ({} days, 3x context of {} days) exceeds 90-day cap; \
                 using 90-day chunks — override with --batch-size if needed",
                min_chunk, context_days
            );
        }

        let reasons: Vec<String> = temporal
            .sources
            .iter()
            .map(|s| format!("{:?}", s))
            .collect();

        BatchSafety::BoundedSafe {
            max_chunk_days,
            context_days,
            reason: if reasons.is_empty() {
                format!("temporal dependency of {} day(s)", context_days)
            } else {
                format!(
                    "temporal dependency of {} day(s) from: {}",
                    context_days,
                    reasons.join(", ")
                )
            },
        }
    }
}

/// Roll F1's unified per-source [`BoundResult`] map into the batched-local
/// three-class **batch-safety** verdict (`batched_models.md` §"Batch safety
/// classification").
///
/// This is the *composition* entry point: everywhere a model's batch-safety
/// class is needed for planning-time refusal or transform selection, it reads
/// this roll-up over the same `BoundResult` map `derive_and_classify_bounds`
/// produces — there is no second, independent walk over the model's SQL to
/// decide safety.
///
/// - All sources `Bounded(_, 0, 0)` (or no timeseries sources at all) →
///   [`BatchSafety::FullyBatchSafe`].
/// - Any source `Bounded` with `before + after > 0` → [`BatchSafety::BoundedSafe`],
///   with `n` (`context_days`, rendered in whole days, rounded up) the maximum
///   `before + after` across every bounded source.
/// - Any source `Unbounded` → [`BatchSafety::PerPartitionOnly`], naming the
///   source(s).
///
/// A `NotDerivable` source is **not** assigned a class — the roll-up refuses
/// with `Err`, naming the offending source, per Constraint 10 ("No silent
/// downgrade to full-refresh"): the optimizer cannot prove the partition
/// DELETE+INSERT contract is safe, so there is nothing to classify. `Err` is
/// checked first, before `Unbounded`/`Bounded`, so a `NotDerivable` source can
/// never be silently outclassed by a coarser-but-still-buildable verdict
/// computed from the *other* sources.
pub fn batch_safety_from_bounds(
    bounds: &HashMap<String, BoundResult>,
) -> Result<BatchSafety, String> {
    // Fail-closed: any NotDerivable source refuses the whole model — checked
    // before Unbounded/Bounded so it can never be masked by another source's
    // coarser-but-buildable verdict.
    let not_derivable: Vec<&str> = bounds
        .iter()
        .filter(|(_, b)| matches!(b, BoundResult::NotDerivable))
        .map(|(name, _)| name.as_str())
        .collect();
    if !not_derivable.is_empty() {
        return Err(format!(
            "cannot derive a temporal bound for source(s) {} — the SQL contains a construct \
             (e.g. a window function without an explicit RANGE BETWEEN INTERVAL clause, or a \
             symbolic month/year interval literal in a bound-relevant position) that cannot be \
             proven safe for the batched partition DELETE+INSERT contract. Rewrite into a \
             derivable form or remove the temporal dependency.",
            not_derivable.join(", ")
        ));
    }

    let unbounded: Vec<&str> = bounds
        .iter()
        .filter(|(_, b)| matches!(b, BoundResult::Unbounded))
        .map(|(name, _)| name.as_str())
        .collect();
    if !unbounded.is_empty() {
        return Ok(BatchSafety::PerPartitionOnly {
            reason: format!(
                "source(s) {} require unbounded history (cumulative-across-history dependency)",
                unbounded.join(", ")
            ),
        });
    }

    let max_context_secs: u64 = bounds
        .values()
        .filter_map(|b| match b {
            BoundResult::Bounded { before, after, .. } => Some(before.0 + after.0),
            _ => None,
        })
        .max()
        .unwrap_or(0);

    if max_context_secs == 0 {
        return Ok(BatchSafety::FullyBatchSafe);
    }

    let context_days = max_context_secs.div_ceil(86400).max(1) as u32;
    let min_chunk = context_days * 3;
    let max_chunk_days = min_chunk.clamp(7, 90);

    let mut reasons: Vec<(&String, u64)> = bounds
        .iter()
        .filter_map(|(name, b)| match b {
            BoundResult::Bounded { before, after, .. } if before.0 + after.0 > 0 => {
                Some((name, before.0 + after.0))
            }
            _ => None,
        })
        .collect();
    reasons.sort_by(|a, b| a.0.cmp(b.0));
    let reason = format!(
        "temporal dependency of {} day(s) from: {}",
        context_days,
        reasons
            .iter()
            .map(|(name, secs)| format!("{name} ({}s)", secs))
            .collect::<Vec<_>>()
            .join(", ")
    );

    Ok(BatchSafety::BoundedSafe {
        max_chunk_days,
        context_days,
        reason,
    })
}

/// Detect incremental materialization opportunity from frontmatter config.
pub fn detect(model: &ModelInfo) -> Result<Option<Opportunity>, String> {
    let inc_config = match &model.incremental_config {
        Some(c) => c,
        None => return Ok(None),
    };

    // timeseries: is required when incremental: is present
    let ts_config = model.timeseries_config.as_ref().ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but no timeseries config",
            model.name
        )
    })?;

    let analysis = analyze_select(&model.sql).ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but SQL could not be parsed",
            model.name
        )
    })?;

    let partition_col = &ts_config.partition_column;
    let event_time_column = &ts_config.event_time_column;
    let overrides = &inc_config.safety_overrides;

    // Validate partition_column alias exists in SELECT list
    let partition_item = analysis.items.iter().find(|item| match item {
        SelectItemKind::GroupByKey { alias, .. } => alias == partition_col,
        SelectItemKind::CountDistinct { alias, .. } => alias == partition_col,
        SelectItemKind::OtherAggregate { alias, .. } => alias == partition_col,
    });

    let partition_item = partition_item.ok_or_else(|| {
        format!(
            "Model '{}': incremental partition_column '{}' not found as alias in SELECT list",
            model.name, partition_col
        )
    })?;

    // Get the expression for the partition column
    let partition_expr = match partition_item {
        SelectItemKind::GroupByKey { text, .. } => text.clone(),
        SelectItemKind::CountDistinct { argument, .. } => argument.clone(),
        SelectItemKind::OtherAggregate { text, .. } => text.clone(),
    };

    // Validate event_time_column is referenced in the SQL
    let stripped_sql = crate::types::Frontmatter::strip(&model.sql);

    // Parsed outer SELECT, shared by the GROUP BY alignment check below and
    // the per-scope HAVING/DISTINCT walk further down — a single parse for
    // all AST-based scope checks.
    let top_select = parse_top_select(stripped_sql);

    // Validate the outer scope's own GROUP BY is a superset of
    // partition_column (only required for aggregate models that have a
    // GROUP BY — per-row models with no GROUP BY are valid for incremental:
    // the partition_column alias in the SELECT list is sufficient, rows are
    // filtered by partition value). Uses the same per-scope
    // `scope_group_by_alignment` signal that licenses HAVING/DISTINCT
    // admission below (`batched_models.md` §"Safety checks") — judged on
    // this scope's own GROUP BY via the AST, not a whole-model text scan
    // (which would misattribute a later UNION branch's GROUP BY to the
    // outer scope).
    if let Some(select) = &top_select {
        if select.group_by_clause().is_some() {
            if let PartitionAlignment::NotAligned { reason } =
                scope_group_by_alignment(select, partition_col)
            {
                return Err(format!(
                    "Model '{}': partition_column '{}' (expression: {}) not found in GROUP BY \
                     clause ({reason})",
                    model.name, partition_col, partition_expr
                ));
            }
        }
    }

    if !stripped_sql.contains(event_time_column.as_str()) {
        return Err(format!(
            "Model '{}': event_time_column '{}' not found in SQL",
            model.name, event_time_column
        ));
    }

    // Validate unique_key columns exist in SELECT list (needed for MERGE strategy)
    let select_aliases: Vec<&str> = analysis
        .items
        .iter()
        .map(|item| match item {
            SelectItemKind::GroupByKey { alias, .. } => alias.as_str(),
            SelectItemKind::CountDistinct { alias, .. } => alias.as_str(),
            SelectItemKind::OtherAggregate { alias, .. } => alias.as_str(),
        })
        .collect();

    for key_col in &inc_config.unique_key {
        if !select_aliases.contains(&key_col.as_str()) {
            return Err(format!(
                "Model '{}': unique_key column '{}' not found as alias in SELECT list",
                model.name, key_col
            ));
        }
    }

    // --- Safety checks ---

    // 2a: Window functions (OVER clause)
    // Partition-aligned OVER is admitted: OVER (PARTITION BY <keys>) where
    // <keys> is a superset of the model's partition_column is safe because the
    // window is partition-local and the DELETE+INSERT contract holds.
    // Any OVER without PARTITION BY, or whose PARTITION BY keys do not include
    // the partition_column, is rejected unless allow_window_functions is set.
    if !overrides.allow_window_functions {
        let upper_sql = stripped_sql.to_uppercase();
        if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
            // Extract every OVER clause and check its PARTITION BY keys.
            if let Some(bad_over) = find_inadmissible_over(stripped_sql, partition_col) {
                return Err(format!(
                    "Model '{}': window function with OVER clause is not compatible with \
                     incremental materialization — window partition keys ({}) do not include \
                     the model's partition_column '{}'. Use OVER (PARTITION BY {} ...) to make \
                     it partition-aligned, or set safety_overrides.allow_window_functions: true",
                    model.name, bad_over, partition_col, partition_col
                ));
            }
        }
    }

    // 2b: HAVING clause — admitted per-scope when that scope's own GROUP BY
    // is a superset of partition_column (`scope_group_by_alignment`,
    // `batched_models.md` §"Safety checks"): the DELETE+INSERT contract
    // deletes and re-inserts whole partitions, so a HAVING that only filters
    // groups already scoped to a single partition cannot see a different
    // group composition on a partial (batched) run than on a full refresh.
    // Walked across every UNION branch (not just the outer SELECT) so a
    // HAVING living in a later branch is judged by *that* branch's own
    // GROUP BY, never the outer/first-branch one.
    if !overrides.allow_having {
        if let Some(select) = &top_select {
            check_having_alignment_all_scopes(select, partition_col, &model.name)?;
        }
    }

    // 2c: LIMIT clause
    if !overrides.allow_limit {
        let upper_sql = stripped_sql.to_uppercase();
        if has_keyword_at_boundary(&upper_sql, "LIMIT") {
            return Err(format!(
                "Model '{}': LIMIT clause is not compatible with incremental materialization \
                 — different time ranges would produce different row subsets",
                model.name
            ));
        }
    }

    // 2d: Subqueries in FROM — parse-based structural gate (replaces the
    // old textual `(` check with an AST walk of the same shape: a derived
    // table nested inside FROM/JOIN). Deliberately **not** extended to
    // WITH-clause CTEs (the "CTE bypass"): existing models such as
    // `examples/web_analytics/models/silver/sessions.sql` already rely on a
    // CTE body whose Form A/B temporal pattern `derive_model_bounds` derives
    // textually, entirely independent of this phase's monotonicity trace —
    // gating on CTE *presence* here (without being able to prove
    // `Traceable`, since that requires the upstream refs' partition
    // columns, only known to `derive_model_source_bounds` via `ModelGraph`)
    // would regress every such model to requiring the override. Instead,
    // `derive_model_source_bounds` classifies CTE bodies unconditionally
    // (see `restrict_ctx_for_derived_tables`): `Traceable` licenses pushdown
    // to the real source, `StaticSeed` is rejected outright, and anything
    // else is left exactly as it was pre-B1 (the existing Form A/B textual
    // walk over the full SQL, CTE body included, is unaffected).
    if !overrides.allow_subqueries {
        if let Some(construct) = find_first_derived_table_construct(stripped_sql) {
            return Err(format!(
                "Model '{}': {construct} is not yet supported with incremental materialization \
                 without safety_overrides.allow_subqueries: true",
                model.name
            ));
        }
    }

    // 2e: Non-deterministic functions — flow/taint check (batched_models.md
    // §"Non-determinism and the equivalence contract"): a non-deterministic
    // value is admitted only when it flows exclusively into a column listed
    // in `batched.nondeterministic_columns`, and never into the
    // event_time_column/partition_column/unique_key roles or a row-set
    // membership/grouping position. See `check_nondeterminism` for the full
    // analysis.
    if !overrides.allow_nondeterministic {
        check_nondeterminism(
            &model.name,
            &analysis,
            stripped_sql,
            partition_col,
            &partition_expr,
            event_time_column,
            inc_config,
        )?;
    }

    // 2f: SELECT DISTINCT — admitted per-scope when partition_column is
    // projected in that scope (`scope_distinct_alignment`): the DISTINCT
    // dedup key is the whole projected row, so it always contains
    // partition_column once projected, and two rows can only collide when
    // they agree on partition_column — i.e. only within the same partition.
    // A DISTINCT computed on a partial (batched) run therefore dedups
    // exactly the same way a full refresh would within that partition.
    // Walked across every UNION branch, same as HAVING.
    if !overrides.allow_distinct {
        if let Some(select) = &top_select {
            check_distinct_alignment_all_scopes(select, partition_col, &model.name)?;
        }
    }

    Ok(Some(Opportunity {
        rule_name: "incremental".to_string(),
        model: model.name.clone(),
        description: format!(
            "Incremental materialization on partition column '{}' (source: '{}', granularity: {:?})",
            partition_col, event_time_column, ts_config.granularity,
        ),
        data: OpportunityData::Incremental {
            event_time_column: event_time_column.clone(),
            partition_column: partition_col.clone(),
            granularity: ts_config.granularity.clone(),
        },
    }))
}

/// Scan `sql` for OVER clauses that are not partition-aligned with `partition_col`.
///
/// Returns `Some(description)` for the first OVER clause whose PARTITION BY
/// keys do not form a superset of `{partition_col}`.
///
/// An OVER clause is admissible when its `PARTITION BY` list contains
/// `partition_col` (case-insensitive, trimmed). An OVER clause with no
/// `PARTITION BY` is inadmissible.
///
/// Returns `None` when every OVER clause in the SQL is admissible (or there
/// are none).
/// Find the first subquery-in-FROM/JOIN construct in `sql`, for the blunt
/// `detect()` structural gate (see call site — deliberately not extended to
/// WITH-clause CTEs). Requires `safety_overrides.allow_subqueries: true`;
/// `derive_model_source_bounds` does the real per-construct monotonicity
/// classification once a model is past this gate.
/// Parse `sql` and return its outermost `SelectStmt`, if it parses to one.
/// Shared entry point for the per-scope `HAVING`/`DISTINCT` alignment walk.
fn parse_top_select(sql: &str) -> Option<smelt_parser::SelectStmt> {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    file.select_stmt()
}

/// Walk `select` and every subsequent UNION branch, rejecting (fail-closed)
/// the first scope whose own `HAVING` is not licensed by
/// `scope_group_by_alignment` — the shared partition-alignment signal
/// (`batched_models.md` §"Safety checks").
fn check_having_alignment_all_scopes(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
    model_name: &str,
) -> Result<(), String> {
    let mut current = select.clone();
    loop {
        if current.having_clause().is_some() {
            if let PartitionAlignment::NotAligned { reason } =
                scope_group_by_alignment(&current, partition_col)
            {
                return Err(format!(
                    "Model '{model_name}': HAVING clause is not compatible with incremental \
                     materialization unless its own GROUP BY includes the partition_column \
                     '{partition_col}' ({reason}) — set safety_overrides.allow_having: true \
                     to override"
                ));
            }
        }
        match current.union_select() {
            Some(next) => current = next,
            None => break,
        }
    }
    Ok(())
}

/// Walk `select` and every subsequent UNION branch, rejecting (fail-closed)
/// the first scope whose own `SELECT DISTINCT` is not licensed by
/// `scope_distinct_alignment`.
fn check_distinct_alignment_all_scopes(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
    model_name: &str,
) -> Result<(), String> {
    let mut current = select.clone();
    loop {
        if current.is_distinct() {
            if let PartitionAlignment::NotAligned { reason } =
                scope_distinct_alignment(&current, partition_col)
            {
                return Err(format!(
                    "Model '{model_name}': SELECT DISTINCT is not compatible with incremental \
                     materialization unless the partition_column '{partition_col}' is projected \
                     in this scope ({reason}) — set safety_overrides.allow_distinct: true to \
                     override"
                ));
            }
        }
        match current.union_select() {
            Some(next) => current = next,
            None => break,
        }
    }
    Ok(())
}

fn find_first_derived_table_construct(sql: &str) -> Option<String> {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;

    if let Some(from_clause) = select.from_clause() {
        for table_ref in from_clause.table_refs() {
            if table_ref.subquery().is_some() {
                let label = table_ref.alias().unwrap_or_else(|| "<unnamed>".to_string());
                return Some(format!("a subquery in FROM ('{label}')"));
            }
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if table_ref.subquery().is_some() {
                    let label = table_ref.alias().unwrap_or_else(|| "<unnamed>".to_string());
                    return Some(format!("a subquery in a JOIN ('{label}')"));
                }
            }
        }
    }

    None
}

fn find_inadmissible_over(sql: &str, partition_col: &str) -> Option<String> {
    let upper_sql = sql.to_uppercase();
    let partition_col_upper = partition_col.to_uppercase();

    let mut search_from = 0;
    while let Some(over_pos) = find_over_keyword(&upper_sql, search_from) {
        // Advance past "OVER" so we don't re-match the same position.
        search_from = over_pos + 4;

        // Skip whitespace after OVER to find the opening '('.
        let rest = &upper_sql[search_from..];
        let paren_offset = match rest.find('(') {
            Some(p) => p,
            None => continue,
        };
        // Ensure only whitespace between OVER and '('.
        let between = &rest[..paren_offset];
        if !between.trim().is_empty() {
            continue;
        }

        let paren_start = search_from + paren_offset; // position of '(' in upper_sql
                                                      // Extract the balanced content inside the OVER (...).
        let over_content = match extract_balanced_parens(&upper_sql, paren_start) {
            Some(c) => c,
            None => continue,
        };
        search_from = paren_start + over_content.len() + 2; // skip '(' content ')'

        // A window carrying a bounded RANGE INTERVAL frame (Form A) is admitted
        // regardless of PARTITION BY alignment: the bounded lookback widens the
        // source read to cover the frame, so the window is partition-local up to
        // that bound and the DELETE+INSERT contract still holds. An UNBOUNDED
        // frame is cumulative-across-history and is NOT made safe this way.
        if has_bounded_range_interval_frame(&over_content) {
            continue;
        }

        // Check for PARTITION BY inside the window spec.
        if let Some(pb_pos) = find_partition_by_in_over(&over_content) {
            let after_pb = &over_content[pb_pos..];
            // Keys end at ORDER BY, ROWS/RANGE/GROUPS, or end of content.
            let keys_text = trim_to_window_clause_end(after_pb);
            // Parse the comma-separated key identifiers.
            let keys: Vec<String> = keys_text
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();

            // Check that partition_col is among the keys.
            let contains_partition_col = keys.iter().any(|k| {
                // Strip any trailing qualifiers (e.g. "t.event_date" → "event_date")
                let bare = k.rsplit('.').next().unwrap_or(k);
                bare.trim() == partition_col_upper
            });

            if !contains_partition_col {
                let key_display: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
                return Some(key_display.join(", "));
            }
        } else {
            // No PARTITION BY at all — inadmissible.
            return Some("<no PARTITION BY>".to_string());
        }
    }
    None
}

/// True if a window spec's frame is a bounded `RANGE BETWEEN INTERVAL '…'
/// PRECEDING [AND …]` (Form A) with no `UNBOUNDED` bound. Such a frame has a
/// finite lookback/lookforward that the source-bound deriver picks up and the
/// planner widens the source read to cover, so the window is partition-local up
/// to that bound — safe for incremental even when PARTITION BY is not aligned to
/// the partition_column. An `UNBOUNDED` bound reads across all history and is
/// deliberately excluded.
fn has_bounded_range_interval_frame(over_content: &str) -> bool {
    let upper = over_content.to_uppercase();
    upper.contains("RANGE BETWEEN") && upper.contains("INTERVAL") && !upper.contains("UNBOUNDED")
}

/// Find the next position of the OVER keyword (word boundary) at or after `from`.
fn find_over_keyword(upper_sql: &str, from: usize) -> Option<usize> {
    let bytes = upper_sql.as_bytes();
    let kw = b"OVER";
    let mut i = from;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == kw {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok = i + 4 >= bytes.len()
                || (!bytes[i + 4].is_ascii_alphanumeric() && bytes[i + 4] != b'_');
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Extract the content inside a balanced `(...)` starting at `paren_pos`
/// (which must point at a `(`). Returns the inner text (excluding outer parens).
fn extract_balanced_parens(sql: &str, paren_pos: usize) -> Option<String> {
    let bytes = sql.as_bytes();
    if paren_pos >= bytes.len() || bytes[paren_pos] != b'(' {
        return None;
    }
    let mut depth = 0usize;
    let mut start = None;
    let mut end = None;
    for (i, &b) in bytes[paren_pos..].iter().enumerate() {
        match b {
            b'(' => {
                depth += 1;
                if depth == 1 {
                    start = Some(paren_pos + i + 1);
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(paren_pos + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let s = start?;
    let e = end?;
    Some(sql[s..e].to_string())
}

/// Find the position (within `over_content`) just after "PARTITION BY ".
/// Returns the offset of the first key character.
fn find_partition_by_in_over(over_content: &str) -> Option<usize> {
    let kw = "PARTITION BY ";
    let upper = over_content.to_uppercase();
    // Find at word boundary
    let mut i = 0;
    while let Some(pos) = upper[i..].find(kw) {
        let abs = i + pos;
        let before_ok = abs == 0 || !upper.as_bytes()[abs - 1].is_ascii_alphanumeric();
        if before_ok {
            return Some(abs + kw.len());
        }
        i = abs + 1;
    }
    // Also try without trailing space (for "PARTITION BY\n")
    let kw2 = "PARTITION BY";
    let mut i = 0;
    while let Some(pos) = upper[i..].find(kw2) {
        let abs = i + pos;
        let before_ok = abs == 0 || !upper.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let after_pos = abs + kw2.len();
        let after_ok =
            after_pos >= upper.len() || !upper.as_bytes()[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            // Skip trailing whitespace
            let skip = upper[after_pos..].len() - upper[after_pos..].trim_start().len();
            return Some(after_pos + skip);
        }
        i = abs + 1;
    }
    None
}

/// Trim a PARTITION BY key list to just the key portion, stopping before
/// ORDER BY, ROWS, RANGE, GROUPS, or end of text.
fn trim_to_window_clause_end(text: &str) -> &str {
    let upper = text.to_uppercase();
    let terminators = ["ORDER BY", "ROWS ", "RANGE ", "GROUPS "];
    let mut end = text.len();
    for term in &terminators {
        if let Some(pos) = upper.find(term) {
            if pos < end {
                end = pos;
            }
        }
    }
    &text[..end]
}

/// Check if a keyword appears at a word boundary in uppercase text.
fn has_keyword_at_boundary(upper_sql: &str, keyword: &str) -> bool {
    let bytes = upper_sql.as_bytes();
    let kw_bytes = keyword.as_bytes();

    for i in 0..bytes.len() {
        if i + kw_bytes.len() > bytes.len() {
            break;
        }
        if &bytes[i..i + kw_bytes.len()] == kw_bytes {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = i + kw_bytes.len() >= bytes.len()
                || !bytes[i + kw_bytes.len()].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// Find the first name in [`NONDETERMINISTIC_FUNCTIONS`] that appears at a
/// word boundary in `text_upper` (already uppercased). Returns the matched
/// function name for use in diagnostics.
fn find_nondeterministic_fn(text_upper: &str) -> Option<&'static str> {
    NONDETERMINISTIC_FUNCTIONS
        .iter()
        .find(|f| has_keyword_at_boundary(text_upper, f))
        .copied()
}

/// True for the run-nondeterministic class (`NOW()`, `CURRENT_TIMESTAMP`,
/// `CURRENT_DATE`) — frozen once per run at compile time, so a direct payload
/// projection carries no cross-run variance risk and is admitted even when
/// the target column is not listed in `nondeterministic_columns` (still
/// subject to the same hard-exclusion roles as every other class).
fn is_run_clock_pinning_fn(func: &str) -> bool {
    classify_function_determinism(func) == FunctionDeterminism::RunDeterministic
}

/// Build the rejection message for a non-deterministic value reaching a
/// position that is deterministic-only regardless of any opt-in.
fn reject_nondeterministic_position(model_name: &str, func: &str, position: &str) -> String {
    format!(
        "Model '{model_name}': non-deterministic function '{func}' is not compatible with \
         incremental materialization — it reaches {position}, which must be deterministic \
         regardless of any `batched.nondeterministic_columns` opt-in"
    )
}

/// Collect the content of every `OVER (...)` window spec in `upper_sql`
/// (already uppercased), regardless of PARTITION BY alignment. Used to check
/// whether a non-deterministic function reaches a window's PARTITION BY /
/// ORDER BY / frame — a hard exclusion independent of the window-alignment
/// check in `find_inadmissible_over` (2a).
fn collect_over_contents(upper_sql: &str) -> Vec<String> {
    let mut contents = Vec::new();
    let mut search_from = 0;
    while let Some(over_pos) = find_over_keyword(upper_sql, search_from) {
        search_from = over_pos + 4;
        let rest = &upper_sql[search_from..];
        let paren_offset = match rest.find('(') {
            Some(p) => p,
            None => continue,
        };
        let between = &rest[..paren_offset];
        if !between.trim().is_empty() {
            continue;
        }
        let paren_start = search_from + paren_offset;
        let over_content = match extract_balanced_parens(upper_sql, paren_start) {
            Some(c) => c,
            None => continue,
        };
        search_from = paren_start + over_content.len() + 2;
        contents.push(over_content);
    }
    contents
}

/// The non-determinism flow/taint check (batched_models.md §"Non-determinism
/// and the equivalence contract"; Constraint 13).
///
/// A non-deterministic function is admitted only when its value flows
/// exclusively into a column listed in `batched.nondeterministic_columns` — a
/// *payload* column, never read back to place, filter, group, or dedup a
/// row. Three hard exclusions reject the value regardless of the opt-in,
/// naming the offending position: the `event_time_column` / `partition_column`
/// expression, any `unique_key` column, and any row-set-membership or
/// grouping position (`WHERE`/`HAVING`/`JOIN … ON`/`DISTINCT`/`GROUP BY`/a
/// window's `PARTITION BY`/`ORDER BY`/frame).
///
/// The run-nondeterministic class (`NOW()`/`CURRENT_*`) is additionally
/// admitted as a direct SELECT-list projection even when the target column is
/// not listed: it is frozen once per run at compile time, so pinning removes
/// the cross-run variance the opt-in exists to gate — there is no analogous
/// exception for the row-nondeterministic class (`RANDOM()`/`UUID()`), which
/// still requires the target column to be listed.
///
/// Anything that cannot be confidently attributed to a single SELECT-list
/// column (nested inside a CTE, a subquery, or an expression combining
/// several things) is rejected — fail closed on indirection.
#[allow(clippy::too_many_arguments)]
fn check_nondeterminism(
    model_name: &str,
    analysis: &crate::analysis::SelectAnalysis,
    stripped_sql: &str,
    partition_col: &str,
    partition_expr: &str,
    event_time_column: &str,
    inc_config: &crate::types::BatchedConfig,
) -> Result<(), String> {
    // 1. Hard exclusion: the partition_column expression.
    if let Some(func) = find_nondeterministic_fn(&partition_expr.to_uppercase()) {
        return Err(reject_nondeterministic_position(
            model_name,
            func,
            &format!("the partition_column '{partition_col}' expression"),
        ));
    }

    // 1. Hard exclusion: the event_time_column expression (when it is itself
    // a computed SELECT-list alias rather than a bare source column).
    if let Some(item) = analysis
        .items
        .iter()
        .find(|i| item_alias(i) == event_time_column)
    {
        if let Some(func) = find_nondeterministic_fn(&item_expr(item).text().to_uppercase()) {
            return Err(reject_nondeterministic_position(
                model_name,
                func,
                &format!("the event_time_column '{event_time_column}' expression"),
            ));
        }
    }

    // 1. Hard exclusion: unique_key columns.
    for key_col in &inc_config.unique_key {
        if let Some(item) = analysis
            .items
            .iter()
            .find(|i| item_alias(i) == key_col.as_str())
        {
            if let Some(func) = find_nondeterministic_fn(&item_expr(item).text().to_uppercase()) {
                return Err(reject_nondeterministic_position(
                    model_name,
                    func,
                    &format!("unique_key column '{key_col}'"),
                ));
            }
        }
    }

    // 2. Hard exclusion: row-set-membership / grouping positions — WHERE,
    // HAVING, JOIN ... ON, GROUP BY.
    if let Some(where_text) = &analysis.where_text {
        if let Some(func) = find_nondeterministic_fn(&where_text.to_uppercase()) {
            return Err(reject_nondeterministic_position(
                model_name,
                func,
                "a WHERE clause",
            ));
        }
    }
    for group_by_expr in &analysis.group_by_exprs {
        if let Some(func) = find_nondeterministic_fn(&group_by_expr.to_uppercase()) {
            return Err(reject_nondeterministic_position(
                model_name,
                func,
                "a GROUP BY key",
            ));
        }
    }

    let parse = smelt_parser::parse(stripped_sql);
    if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
        if let Some(select) = file.select_stmt() {
            if let Some(having) = select.having_clause() {
                if let Some(expr) = having.expression() {
                    if let Some(func) = find_nondeterministic_fn(&expr.text().to_uppercase()) {
                        return Err(reject_nondeterministic_position(
                            model_name,
                            func,
                            "a HAVING clause",
                        ));
                    }
                }
            }
            if let Some(from_clause) = select.from_clause() {
                for join in from_clause.joins() {
                    if let Some(condition) = join.condition() {
                        if let Some(on_expr) = condition.on_expression() {
                            if let Some(func) =
                                find_nondeterministic_fn(&on_expr.text().to_uppercase())
                            {
                                return Err(reject_nondeterministic_position(
                                    model_name,
                                    func,
                                    "a JOIN ... ON clause",
                                ));
                            }
                        }
                    }
                }
            }

            // 2. Hard exclusion: any CTE body. None of the clause-scoped checks
            // above descend into `WITH ... AS (...)` bodies — they only look at
            // the outer `SelectStmt`'s own WHERE/GROUP BY/HAVING/JOIN clauses.
            // A CTE's WHERE clause can filter row-set membership just as much
            // as the outer query's WHERE, so a non-deterministic function
            // anywhere inside a CTE body is rejected outright, fail-closed —
            // regardless of what the outer query does with the CTE's output or
            // whether the same function name is also used safely elsewhere
            // (e.g. an outer `NOW() AS inserted_at` payload projection does not
            // excuse a `WHERE NOW() - event_ts < INTERVAL '1 day'` inside a CTE).
            if let Some(with_clause) = select.with_clause() {
                for cte in with_clause.ctes() {
                    let cte_text_upper = cte.syntax().text().to_string().to_uppercase();
                    if let Some(func) = find_nondeterministic_fn(&cte_text_upper) {
                        let cte_name = cte.name().unwrap_or_else(|| "<unnamed>".to_string());
                        return Err(reject_nondeterministic_position(
                            model_name,
                            func,
                            &format!("a CTE ('{cte_name}') body"),
                        ));
                    }
                }
            }
        }
    }

    // 2. Hard exclusion: a window's PARTITION BY / ORDER BY / frame.
    let upper_sql = stripped_sql.to_uppercase();
    for over_content in collect_over_contents(&upper_sql) {
        if let Some(func) = find_nondeterministic_fn(&over_content) {
            return Err(reject_nondeterministic_position(
                model_name,
                func,
                "a window's PARTITION BY / ORDER BY / frame",
            ));
        }
    }

    // 2. Hard exclusion: SELECT DISTINCT — the whole row is the dedup key, so
    // any non-deterministic value in the SELECT list reaches a
    // row-set-membership position regardless of which column it lands in.
    let trimmed_upper = stripped_sql.trim().to_uppercase();
    let is_distinct_select = trimmed_upper.starts_with("SELECT DISTINCT")
        || trimmed_upper.starts_with("SELECT  DISTINCT")
        || trimmed_upper.contains("\nSELECT DISTINCT");

    // 3/4. Direct SELECT-list projection: admitted when the value flows only
    // into a listed payload column (or, for the run-nondeterministic class,
    // any direct projection — pinning removes the cross-run variance risk).
    for item in &analysis.items {
        let alias = item_alias(item);
        let expr_upper = item_expr(item).text().to_uppercase();
        if let Some(func) = find_nondeterministic_fn(&expr_upper) {
            if is_distinct_select {
                return Err(reject_nondeterministic_position(
                    model_name,
                    func,
                    "SELECT DISTINCT (the whole row is the dedup key)",
                ));
            }
            let listed = inc_config
                .nondeterministic_columns
                .iter()
                .any(|c| c == alias);
            if listed || is_run_clock_pinning_fn(func) {
                continue;
            }
            return Err(format!(
                "Model '{model_name}': non-deterministic function '{func}' flows into column \
                 '{alias}', which is not listed in `batched.nondeterministic_columns` — add \
                 '{alias}' to `batched.nondeterministic_columns` to accept the variation, or set \
                 `safety_overrides.allow_nondeterministic: true`"
            ));
        }
    }

    // 5. Fail closed on indirection: any remaining occurrence not
    // attributable to a single SELECT-list column (nested inside a CTE, a
    // subquery, or an expression this analysis cannot confidently attribute).
    for func in NONDETERMINISTIC_FUNCTIONS {
        if has_keyword_at_boundary(&upper_sql, func) {
            let accounted_for = analysis
                .items
                .iter()
                .any(|item| has_keyword_at_boundary(&item_expr(item).text().to_uppercase(), func));
            if !accounted_for {
                return Err(format!(
                    "Model '{model_name}': non-deterministic function '{func}' is used in a \
                     position this analysis cannot attribute to a single SELECT-list column \
                     (e.g. inside a CTE, subquery, or derived-table expression) — incremental \
                     materialization requires the non-determinism to flow directly into one \
                     listed payload column; rewrite the model so the call is a direct \
                     SELECT-list projection, or set `safety_overrides.allow_nondeterministic: \
                     true`"
                ));
            }
        }
    }

    Ok(())
}

/// Derive per-source bounds *and* their pushdown-depth injection point for an
/// incremental model, in a single downward walk over the model's SQL.
///
/// Builds a `BoundContext` from the model's upstream refs (only those with
/// `timeseries:` declared are included; lookup sources are skipped), then
/// derives each source's [`BoundResult`] from `derive_model_bounds` exactly
/// once. The same call that produces the bound also classifies its
/// [`InjectionPoint`] (`BoundResult::injection_point`), so the value used to
/// widen the per-source scan and the value that decides whether the outer
/// output clamp is still needed can never disagree — there is no second,
/// independent derivation of either window (research
/// `20260703-model-updates.md` §3.2/§3.5).
///
/// Also returns `Err(diagnostic)` when any bound is `NotDerivable` — that
/// triggers the same refusal path as a failed safety check (fail-closed,
/// unchanged from before this walk was unified).
pub fn derive_model_source_bounds(
    model: &ModelInfo,
    graph: &ModelGraph,
) -> Result<HashMap<String, (InjectionPoint, BoundResult)>, String> {
    // Build the context: map ref name → partition_column for timeseries refs.
    let mut ctx = BoundContext::new();
    for ref_name in &model.refs {
        if let Some(upstream) = graph.get(ref_name) {
            if let Some(ts) = &upstream.timeseries_config {
                ctx.add_source(ref_name, &ts.partition_column);
            }
            // Lookup sources (no timeseries) are silently skipped.
        }
    }

    let stripped = crate::types::Frontmatter::strip(&model.sql);

    // Extend the derivation to look inside UNION branches, subquery/CTE
    // bodies, and join inputs (event-time monotonicity trace consumers,
    // `batched_models.md` §"Event-time monotonicity trace") rather than only
    // the single outer SELECT's FROM clause. `restrict_ctx_for_constructs`
    // narrows `ctx` down to the sources whose pushdown is actually licensed
    // by a `Traceable` verdict for the construct that references them —
    // `derive_model_bounds` below still does the existing per-source Form
    // A/B textual bound walk, just over the (possibly narrowed) source set.
    let ctx = restrict_ctx_for_constructs(model, &ctx, stripped)?;

    let bounds = crate::analysis::source_bounds::derive_and_classify_bounds(stripped, &ctx);

    // Check for NotDerivable — refuse the model.
    for (source_name, (_, bound)) in &bounds {
        if *bound == BoundResult::NotDerivable {
            return Err(format!(
                "Model '{}': cannot derive a temporal bound for source '{}'. \
                 The SQL contains a window function (e.g. LAG/LEAD) without an explicit \
                 RANGE BETWEEN INTERVAL clause. Rewrite using Form A \
                 (RANGE BETWEEN INTERVAL '...' PRECEDING) or Form B \
                 (WHERE col BETWEEN expr - INTERVAL '...' AND expr) to make the \
                 lookback derivable, or remove the temporal dependency.",
                model.name, source_name
            ));
        }
    }

    Ok(bounds)
}

/// Narrow `ctx` (all upstream timeseries refs) down to the sources whose
/// pushdown is actually licensed by the model's event-time monotonicity
/// trace, when the model's outer SELECT is a UNION, has a join over more
/// than one candidate timeseries source, or projects its event-time column
/// through a subquery/CTE. Returns `ctx` unchanged (cloned) when none of
/// those constructs apply — the existing single-SELECT, single-source
/// behaviour is untouched.
fn restrict_ctx_for_constructs(
    model: &ModelInfo,
    ctx: &BoundContext,
    stripped_sql: &str,
) -> Result<BoundContext, String> {
    let unchanged = || BoundContext {
        source_partition_cols: ctx.source_partition_cols.clone(),
    };

    let Some(ts_config) = &model.timeseries_config else {
        return Ok(unchanged());
    };
    let parse = smelt_parser::parse(stripped_sql);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return Ok(unchanged());
    };
    let Some(select) = file.select_stmt() else {
        return Ok(unchanged());
    };
    let Some(items) = select_stmt_items(&select) else {
        return Ok(unchanged());
    };
    let Some(position) = items
        .iter()
        .position(|item| item_alias(item) == ts_config.partition_column)
    else {
        // `detect()` already validated this for models that go through it;
        // `derive_model_source_bounds` can also be called directly (e.g. for
        // `smelt explain`) — conservatively leave ctx untouched.
        return Ok(unchanged());
    };

    // UNION ALL: trace each branch independently.
    if select.has_union() && select.is_union_all() {
        return restrict_ctx_for_union(model, &select, &ts_config.partition_column, position, ctx);
    }

    // JOIN: only resolve a driving fact when the FROM clause actually joins
    // more than one candidate timeseries source — otherwise there is no
    // ambiguity to resolve and the existing single-source path is unchanged.
    if let Some(from_clause) = select.from_clause() {
        let alias_sources = from_clause_alias_sources(&from_clause);
        let candidate_sources: HashSet<&String> = alias_sources
            .iter()
            .map(|(_, source)| source)
            .filter(|source| ctx.source_partition_cols.contains_key(source.as_str()))
            .collect();
        if from_clause.joins().next().is_some() && candidate_sources.len() > 1 {
            let expr = item_expr(&items[position]).clone();
            return restrict_ctx_for_join(model, &expr, &alias_sources, ctx);
        }
    }

    // Subquery/CTE bodies.
    restrict_ctx_for_derived_tables(model, &select, &ts_config.partition_column, ctx)
}

/// Trace every branch of a UNION-ALL `select` for the item matched by
/// `target_col` (by alias, falling back to ordinal `position` — UNION output
/// column names come from branch 1 only), scoping each branch's trace to
/// just its own FROM-clause source(s) when resolvable (`smelt.<path>` refs)
/// — different branches commonly share a partition-column *name* across
/// distinct sources (e.g. every event table calls it `event_date`), which
/// the full-model `ctx` alone cannot disambiguate (a name-based match would
/// see every such source and fail closed on ambiguity). Falls back to the
/// full `ctx` when a branch's FROM clause isn't in `smelt.<path>` form
/// (nothing to scope by).
///
/// Returns the ordered per-branch verdicts (branch 1 first) rather than
/// deciding a policy for a non-`Traceable` one — that decision differs by
/// caller: pushdown scoping (`restrict_ctx_for_union`) treats `NotTraceable`
/// as "stay at the outer clamp" and rejects only `StaticSeed`, while
/// outer-select injectability checking
/// (`rule_diagnostics::check_event_time_injectable`) fails closed on
/// *either* verdict. Factoring the branch walk out here means the two
/// policies can never disagree about what a given branch actually traces to
/// (design decision 1, Phase B1; diagnostic-parity extension).
pub fn trace_union_branches(
    select: &smelt_parser::SelectStmt,
    target_col: &str,
    position: usize,
    ctx: &BoundContext,
) -> Result<Vec<EventTimeTrace>, String> {
    let mut traces = Vec::new();
    let mut current = select.clone();
    let mut branch_no = 1usize;
    loop {
        let items = select_stmt_items(&current)
            .ok_or_else(|| format!("UNION branch {branch_no} has no SELECT list"))?;
        let expr =
            find_item_expr_by_alias_or_position(&items, target_col, position).ok_or_else(|| {
                format!(
                    "UNION branch {branch_no} has no item at the '{target_col}' position to trace"
                )
            })?;
        let branch_ctx = current.from_clause().and_then(|from_clause| {
            let alias_sources = from_clause_alias_sources(&from_clause);
            let mut scoped = BoundContext::new();
            for (_, source_name) in &alias_sources {
                if let Some(partition_col) = ctx.source_partition_cols.get(source_name) {
                    scoped.add_source(source_name, partition_col);
                }
            }
            if scoped.source_partition_cols.is_empty() {
                None
            } else {
                Some(scoped)
            }
        });
        traces.push(trace_event_time(&expr, branch_ctx.as_ref().unwrap_or(ctx)));
        match current.union_select() {
            Some(next) => {
                current = next;
                branch_no += 1;
            }
            None => break,
        }
    }
    Ok(traces)
}

/// UNION ALL branch tracing (design decision 1, Phase B1): trace each
/// branch's item at `position` via [`trace_union_branches`]. An
/// all-`Traceable` set unlocks pushdown to each branch's named source
/// (merged into one restricted ctx); a `StaticSeed` branch is the NULL/constant
/// hazard and is rejected, naming the branch and construct; a `NotTraceable`
/// branch is the conservative "stay at the outer clamp" case for the union
/// as a whole.
fn restrict_ctx_for_union(
    model: &ModelInfo,
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
    position: usize,
    ctx: &BoundContext,
) -> Result<BoundContext, String> {
    let traces = trace_union_branches(select, partition_col, position, ctx)
        .map_err(|e| format!("Model '{}': {e}", model.name))?;
    let mut new_ctx = BoundContext::new();
    for (i, trace) in traces.into_iter().enumerate() {
        let branch_no = i + 1;
        match trace {
            EventTimeTrace::Traceable {
                source,
                source_column,
                ..
            } => {
                new_ctx.add_source(&source, &source_column);
            }
            EventTimeTrace::StaticSeed { reason } => {
                return Err(format!(
                    "Model '{}': UNION branch {branch_no} projects a static/NULL seed for \
                     '{partition_col}' ({reason}) — not a partitionable stream, so batched \
                     pushdown cannot be proven safe for this UNION",
                    model.name
                ));
            }
            EventTimeTrace::NotTraceable { .. } => {
                // Conservative: no per-source pushdown licensed by this
                // UNION; stay at the outer clamp for the whole model.
                return Ok(BoundContext::new());
            }
        }
    }
    Ok(new_ctx)
}

/// Join driving-fact resolution (design decision 3, Phase B1): resolve the
/// single driving fact among a join's timeseries inputs via
/// `resolve_join_driving_fact` (alias-scoped leaf resolution), then restrict
/// `ctx` to just that source — every other joined source gets no bound
/// entry (full scan, matching how lookup sources already behave when absent
/// from `ctx`), so the misfilter hazard of pushing a filter onto a
/// non-driving lookup input is zero by construction.
///
/// Reads `model.timeseries_config`'s declared-monotonicity escape hatch
/// (`assert_monotonic`, DC1) and threads it into the trace: an otherwise
/// `NotTraceable{Undecidable}` driving fact (an opaque UDF wrapping the
/// event-time column) is admitted when declared; a positive disproof
/// (`StaticSeed`, or `NotTraceable{Disproven}`) is refused exactly as before
/// regardless of the declaration.
fn restrict_ctx_for_join(
    model: &ModelInfo,
    event_time_expr: &smelt_parser::Expr,
    alias_sources: &[(String, String)],
    ctx: &BoundContext,
) -> Result<BoundContext, String> {
    let declared_monotonic = model
        .timeseries_config
        .as_ref()
        .is_some_and(|ts| ts.assert_monotonic);
    match resolve_join_driving_fact(event_time_expr, alias_sources, ctx, declared_monotonic) {
        EventTimeTrace::Traceable {
            source,
            source_column,
            ..
        } => Ok(BoundContext::new().with_source(&source, &source_column)),
        EventTimeTrace::StaticSeed { reason } => Err(format!(
            "Model '{}': join driving-fact resolution hit a static/NULL seed ({reason}) — not a \
             partitionable stream",
            model.name
        )),
        EventTimeTrace::NotTraceable { reason, .. } => Err(format!(
            "Model '{}': cannot resolve a single driving fact among this model's joined \
             timeseries sources ({reason}); batched pushdown requires exactly one join input to \
             trace the event-time projection back to its source partition column",
            model.name
        )),
    }
}

/// Subquery/CTE body tracing (design decision 2, Phase B1): classify every
/// derived table (subquery in FROM/JOIN, or a WITH-clause CTE) whose own
/// SELECT list projects an item aliased `partition_col`. `Traceable`
/// licenses pushdown to the real underlying source (already present in
/// `ctx`, so no narrowing needed); `StaticSeed` is rejected outright —
/// unconditionally, not just when `allow_subqueries` is unset, since a
/// static/NULL seed is a genuine correctness hazard, not a safety
/// trade-off. `NotTraceable` is a no-op: it neither narrows nor rejects,
/// leaving `ctx` exactly as it already was. This preserves the pre-B1
/// behaviour for CTE bodies that were never gated at all (the "CTE
/// bypass") and already derive a bound via the existing Form A/B textual
/// walk in `derive_model_bounds` — e.g.
/// `examples/web_analytics/models/silver/sessions.sql`'s `sessionized` CTE
/// projects `session_start_date` from a `TableExpr`-returning function
/// call, which this trace cannot resolve to a source partition column
/// (`NotTraceable`, not `StaticSeed`), but the model's own `WHERE ...
/// BETWEEN session_start_date - INTERVAL '1 day' AND ... + INTERVAL '1
/// day'` clause already gives `derive_model_bounds` everything it needs —
/// gating that model on this trace's inability to prove monotonicity would
/// be a regression, not a safety improvement. A derived table whose body
/// doesn't project `partition_col` at all is not relevant to event-time
/// tracing and is likewise left untouched.
fn restrict_ctx_for_derived_tables(
    model: &ModelInfo,
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
    ctx: &BoundContext,
) -> Result<BoundContext, String> {
    let mut derived: Vec<(String, EventTimeTrace)> = Vec::new();

    if let Some(with_clause) = select.with_clause() {
        for cte in with_clause.ctes() {
            let name = cte.name().unwrap_or_else(|| "<unnamed>".to_string());
            if let Some(inner) = cte.query().and_then(|q| q.select_stmt()) {
                if let Some(items) = select_stmt_items(&inner) {
                    if let Some(item) = items.iter().find(|i| item_alias(i) == partition_col) {
                        let expr = item_expr(item).clone();
                        derived.push((format!("CTE '{name}'"), trace_event_time(&expr, ctx)));
                    }
                }
            }
        }
    }

    if let Some(from_clause) = select.from_clause() {
        for table_ref in from_clause.table_refs() {
            if let Some(subq) = table_ref.subquery() {
                if let Some(inner) = subq.select_stmt() {
                    if let Some(items) = select_stmt_items(&inner) {
                        if let Some(item) = items.iter().find(|i| item_alias(i) == partition_col) {
                            let expr = item_expr(item).clone();
                            let label =
                                table_ref.alias().unwrap_or_else(|| "<unnamed>".to_string());
                            derived.push((
                                format!("subquery '{label}'"),
                                trace_event_time(&expr, ctx),
                            ));
                        }
                    }
                }
            }
        }
        for join in from_clause.joins() {
            let Some(table_ref) = join.table_ref() else {
                continue;
            };
            let Some(subq) = table_ref.subquery() else {
                continue;
            };
            let Some(inner) = subq.select_stmt() else {
                continue;
            };
            let Some(items) = select_stmt_items(&inner) else {
                continue;
            };
            if let Some(item) = items.iter().find(|i| item_alias(i) == partition_col) {
                let expr = item_expr(item).clone();
                let label = table_ref.alias().unwrap_or_else(|| "<unnamed>".to_string());
                derived.push((
                    format!("joined subquery '{label}'"),
                    trace_event_time(&expr, ctx),
                ));
            }
        }
    }

    if derived.is_empty() {
        // No derived table projects the event-time column — nothing to
        // classify; keep ctx untouched (unchanged behaviour).
        return Ok(BoundContext {
            source_partition_cols: ctx.source_partition_cols.clone(),
        });
    }

    let new_ctx = BoundContext {
        source_partition_cols: ctx.source_partition_cols.clone(),
    };
    for (label, trace) in derived {
        match trace {
            EventTimeTrace::Traceable { .. } => {
                // Already licensed via the existing ctx entry for that source.
            }
            EventTimeTrace::StaticSeed { reason } => {
                return Err(format!(
                    "Model '{}': {label} projects a static/NULL seed for '{partition_col}' \
                     ({reason}) — not a partitionable stream; batched pushdown cannot be proven \
                     safe",
                    model.name
                ));
            }
            EventTimeTrace::NotTraceable { reason, .. } => {
                tracing::debug!(
                    model = %model.name,
                    %label,
                    %reason,
                    "derived table does not trace to a source partition column; \
                     leaving ctx unrestricted (falls back to the existing Form A/B textual walk)"
                );
            }
        }
    }
    Ok(new_ctx)
}

/// Produce a SetIncremental transformation for a model.
pub fn optimize(model: &ModelInfo) -> Result<Option<Transformation>, String> {
    let opportunity = detect(model)?;
    match opportunity {
        None => Ok(None),
        Some(opp) => match &opp.data {
            OpportunityData::Incremental {
                event_time_column,
                partition_column,
                granularity,
            } => Ok(Some(Transformation::SetIncremental {
                model: model.name.clone(),
                event_time_column: event_time_column.clone(),
                partition_column: partition_column.clone(),
                granularity: granularity.clone(),
            })),
            _ => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::TimeseriesConfig;
    use crate::types::{BatchedConfig, BatchedSafetyOverrides, Granularity};

    fn model(name: &str, sql: &str, partition_column: &str) -> ModelInfo {
        model_with_event_time(name, sql, partition_column, "event_timestamp")
    }

    fn model_with_event_time(
        name: &str,
        sql: &str,
        partition_column: &str,
        event_time_column: &str,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: event_time_column.to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        }
    }

    fn model_with_overrides(
        name: &str,
        sql: &str,
        partition_column: &str,
        overrides: BatchedSafetyOverrides,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: overrides,
            }),
        }
    }

    /// Build a model with a `batched.nondeterministic_columns` list (and
    /// optional `unique_key`) for the non-determinism flow/taint tests.
    fn model_with_nondeterministic_columns(
        name: &str,
        sql: &str,
        partition_column: &str,
        nondeterministic_columns: Vec<String>,
        unique_key: Vec<String>,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key,
                nondeterministic_columns,
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        }
    }

    #[test]
    fn test_detect_incremental() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref event_time_column,
                ref partition_column,
                ref granularity,
            } => {
                assert_eq!(event_time_column, "event_timestamp");
                assert_eq!(partition_column, "event_date");
                assert_eq!(granularity, &Granularity::Day);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_with_explicit_event_time_column() {
        let m = model_with_event_time(
            "daily",
            "SELECT date_trunc('day', my_ts) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
            "my_ts",
        );
        let opp = detect(&m).unwrap().unwrap();
        match opp.data {
            OpportunityData::Incremental {
                ref event_time_column,
                ..
            } => {
                assert_eq!(event_time_column, "my_ts");
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_event_time_column_not_in_sql() {
        let m = model_with_event_time(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
            "nonexistent_column",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in SQL"));
    }

    #[test]
    fn test_detect_with_granularity() {
        use crate::graph::TimeseriesConfig;
        let m = ModelInfo {
            name: "hourly".to_string(),
            sql: "SELECT date_trunc('hour', event_timestamp) as event_hour, COUNT(*) as cnt FROM events GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_hour".to_string(),
                granularity: Granularity::Hour,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let opp = detect(&m).unwrap().unwrap();
        match opp.data {
            OpportunityData::Incremental {
                ref granularity, ..
            } => {
                assert_eq!(granularity, &Granularity::Hour);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_no_config() {
        let m = ModelInfo {
            name: "test".to_string(),
            sql: "SELECT a FROM t GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: None,
            incremental_config: None,
        };
        assert!(detect(&m).unwrap().is_none());
    }

    #[test]
    fn test_detect_invalid_partition_column() {
        let m = model("test", "SELECT a FROM t GROUP BY 1", "nonexistent_column");
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found as alias"));
    }

    #[test]
    fn test_detect_rejects_window_functions() {
        let m = model(
            "windowed",
            "SELECT date_trunc('day', event_timestamp) as event_date, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_timestamp) as rn FROM events GROUP BY 1",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("window function"));
    }

    #[test]
    fn test_detect_allows_window_functions_with_override() {
        let m = model_with_overrides(
            "windowed",
            "SELECT date_trunc('day', event_timestamp) as event_date, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_timestamp) as rn FROM events GROUP BY 1",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_over_with_bounded_range_interval_frame_is_admissible() {
        // A window whose PARTITION BY does NOT include the partition_column is
        // still partition-local when it carries a bounded RANGE INTERVAL frame:
        // the Form A lookback widens the source read to cover the frame, so the
        // DELETE+INSERT contract holds. It must be admitted without an override.
        let sql = "SELECT MAX(boundary_ts) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS s FROM events";
        assert_eq!(find_inadmissible_over(sql, "session_start_date"), None);
    }

    #[test]
    fn test_over_without_frame_still_inadmissible() {
        // No frame, non-aligned PARTITION BY → still inadmissible (bare window
        // function reads unbounded prior rows within the partition).
        let sql =
            "SELECT ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY event_ts) AS rn FROM events";
        assert!(find_inadmissible_over(sql, "session_start_date").is_some());
    }

    #[test]
    fn test_over_with_unbounded_range_still_inadmissible() {
        // An UNBOUNDED PRECEDING frame is genuinely cumulative-across-history and
        // is not made safe by the lookback widening — still inadmissible.
        let sql = "SELECT SUM(x) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS s FROM events";
        assert!(find_inadmissible_over(sql, "session_start_date").is_some());
    }

    /// A `HAVING` whose own scope's `GROUP BY` is a superset of
    /// `partition_column` is group-aligned and must build without an
    /// override (`batched_models.md` §"Safety checks").
    #[test]
    fn test_detect_admits_group_aligned_having() {
        let m = model(
            "having_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 HAVING COUNT(*) > 10",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert!(result.unwrap().is_some());
    }

    /// A `HAVING` living in a UNION branch whose *own* `GROUP BY` omits
    /// `partition_column` must still refuse — fail-closed — even though the
    /// first branch (and the outer alignment check that runs on it) is
    /// aligned. Proves the alignment verdict is computed per-scope, not just
    /// on the outer/first-branch `GROUP BY`.
    #[test]
    fn test_detect_rejects_having_not_group_aligned_in_union_branch() {
        let m = model_with_event_time(
            "having_union_model",
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events_a GROUP BY 1, 2 \
             UNION ALL \
             SELECT event_date, user_id, COUNT(*) as cnt FROM events_b GROUP BY 2 HAVING COUNT(*) > 10",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err(), "expected Err, got {:?}", result);
        let err = result.unwrap_err();
        assert!(err.contains("HAVING"), "err: {err}");
    }

    #[test]
    fn test_detect_allows_having_with_override() {
        let m = model_with_overrides(
            "having_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 HAVING COUNT(*) > 10",
            "event_date",
            BatchedSafetyOverrides {
                allow_having: true,
                ..Default::default()
            },
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_detect_rejects_limit() {
        let m = model(
            "limited",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 LIMIT 100",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("LIMIT"));
    }

    #[test]
    fn test_detect_rejects_subquery() {
        let m = model(
            "subquery_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM (SELECT * FROM events WHERE active = true) sub GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("subqueries"));
    }

    #[test]
    fn test_detect_rejects_nondeterministic() {
        let m = model(
            "random_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, RANDOM() as r, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"));
        assert!(err.contains("RANDOM"));
    }

    // --- Non-determinism flow/taint tests (batched.nondeterministic_columns) ---

    #[test]
    fn test_nondeterministic_now_listed_column_admitted() {
        // NOW() flowing only into a listed payload column builds cleanly.
        let m = model_with_nondeterministic_columns(
            "audit_stamped",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt, \
             NOW() as inserted_at FROM events GROUP BY 1, 2",
            "event_date",
            vec!["inserted_at".to_string()],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_nondeterministic_now_unlisted_column_admitted_pinned_clock() {
        // Run-clock pinning: NOW() as a direct SELECT-list projection is
        // admitted even when the target column is NOT listed in
        // nondeterministic_columns — it is frozen once per run, so there is
        // no cross-run variance for the guardrail to gate.
        let m = model_with_nondeterministic_columns(
            "audit_stamped_unlisted",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt, \
             NOW() as inserted_at FROM events GROUP BY 1, 2",
            "event_date",
            vec![],
            vec![],
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "expected Ok (pinned clock), got {:?}",
            result
        );
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_nondeterministic_now_in_cte_where_rejects_despite_outer_pinned_projection() {
        // Fail-closed-on-indirection: the outer SELECT's `NOW() as inserted_at`
        // is a legitimately admitted pinned-clock projection (see the
        // `_pinned_clock` test above), but a *separate* NOW() call inside a
        // CTE's own WHERE clause varies row-set membership across separate
        // `smelt run` invocations. The catch-all must not treat the CTE
        // occurrence as "accounted for" just because the same function name
        // appears safely elsewhere in the outer SELECT list.
        let m = model_with_nondeterministic_columns(
            "cte_now_in_where",
            "WITH staged AS (\
                 SELECT event_timestamp, user_id FROM events \
                 WHERE NOW() - event_timestamp < INTERVAL '1 day'\
             ) \
             SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt, \
             NOW() as inserted_at FROM staged GROUP BY 1, 2",
            "event_date",
            vec!["inserted_at".to_string()],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_err(), "expected Err, got {:?}", result);
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"), "err: {err}");
        assert!(err.contains("CTE"), "err: {err}");
    }

    #[test]
    fn test_nondeterministic_random_where_rejects() {
        let m = model_with_nondeterministic_columns(
            "random_in_where",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt \
             FROM events WHERE RANDOM() > 0.5 GROUP BY 1, 2",
            "event_date",
            vec![],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"), "err: {err}");
        assert!(err.contains("WHERE"), "err: {err}");
    }

    #[test]
    fn test_nondeterministic_random_group_by_rejects() {
        let m = model_with_nondeterministic_columns(
            "random_in_group_by",
            "SELECT date_trunc('day', event_timestamp) as event_date, RANDOM() as grp, \
             COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
            vec![],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"), "err: {err}");
        assert!(err.contains("GROUP BY"), "err: {err}");
    }

    #[test]
    fn test_nondeterministic_random_partition_by_window_rejects() {
        // RANDOM() aligned so the 2a window-alignment check admits it (the
        // PARTITION BY keys are a superset that includes the partition
        // column), isolating the assertion to the non-determinism check.
        let m = model_with_nondeterministic_columns(
            "random_in_window_partition_by",
            "SELECT date_trunc('day', event_timestamp) as event_date, \
             SUM(amount) OVER (PARTITION BY event_date, RANDOM() ORDER BY event_timestamp) \
             as running_total FROM events",
            "event_date",
            vec![],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"), "err: {err}");
        assert!(err.contains("PARTITION BY"), "err: {err}");
    }

    #[test]
    fn test_nondeterministic_random_unlisted_column_still_rejects() {
        // Flow/taint case: RANDOM() flowing into a column NOT listed in
        // nondeterministic_columns is still rejected (unchanged from the
        // pre-opt-in behaviour) — only the run-nondeterministic class gets
        // the unlisted pinned-clock exception.
        let m = model_with_nondeterministic_columns(
            "random_unlisted",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt, \
             RANDOM() as foo FROM events GROUP BY 1, 2",
            "event_date",
            vec![],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"), "err: {err}");
        assert!(err.contains("foo") || err.contains("RANDOM"), "err: {err}");
    }

    #[test]
    fn test_nondeterministic_random_listed_column_admitted() {
        // The opt-in also covers the row-nondeterministic class when the
        // target column is explicitly listed.
        let m = model_with_nondeterministic_columns(
            "random_listed",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt, \
             RANDOM() as foo FROM events GROUP BY 1, 2",
            "event_date",
            vec!["foo".to_string()],
            vec![],
        );
        let result = detect(&m);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert!(result.unwrap().is_some());
    }

    /// A `SELECT DISTINCT` that projects `partition_column` is aligned (the
    /// dedup key is the whole row, which always contains partition_column
    /// once projected) and must build without an override.
    #[test]
    fn test_detect_admits_group_aligned_distinct() {
        let m = model(
            "distinct_model",
            "SELECT DISTINCT date_trunc('day', event_timestamp) as event_date, user_id FROM events GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert!(result.unwrap().is_some());
    }

    /// A `DISTINCT` living in a UNION branch that does *not* project
    /// `partition_column` in its own select list must still refuse —
    /// fail-closed — even though the outer/first branch does project it.
    /// Proves the alignment verdict is computed per-scope.
    #[test]
    fn test_detect_rejects_distinct_not_aligned_in_union_branch() {
        let m = model_with_event_time(
            "distinct_union_model",
            "SELECT event_date, user_id FROM events_a \
             UNION ALL \
             SELECT DISTINCT user_id FROM events_b",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err(), "expected Err, got {:?}", result);
        let err = result.unwrap_err();
        assert!(err.contains("DISTINCT"), "err: {err}");
    }

    #[test]
    fn test_detect_allows_count_distinct() {
        // COUNT(DISTINCT ...) inside an aggregate should NOT trigger the DISTINCT check
        let m = model(
            "count_distinct_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, COUNT(DISTINCT user_id) as unique_users FROM events GROUP BY 1",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_optimize_produces_transformation() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let t = optimize(&m).unwrap().unwrap();
        match t {
            Transformation::SetIncremental {
                model,
                event_time_column,
                partition_column,
                granularity,
            } => {
                assert_eq!(model, "daily");
                assert_eq!(event_time_column, "event_timestamp");
                assert_eq!(partition_column, "event_date");
                assert_eq!(granularity, Granularity::Day);
            }
            _ => panic!("Expected SetIncremental"),
        }
    }

    #[test]
    fn test_detect_with_weekly_granularity() {
        use smelt_core::config::Weekday;
        let m = ModelInfo {
            name: "weekly".to_string(),
            sql: "SELECT date_trunc('week', event_timestamp) as event_week, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_week".to_string(),
                granularity: Granularity::Week,
                week_start: Some(Weekday::Monday),
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref granularity, ..
            } => {
                assert_eq!(granularity, &Granularity::Week);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    // --- Per-row (non-aggregate) model tests ---

    /// A per-row SELECT (no GROUP BY) with `partition_column` in the SELECT
    /// list must be accepted by `detect()`.  The Phase 2 fix skips the
    /// GROUP-BY membership check when `group_by_exprs` is empty.
    #[test]
    fn test_detect_per_row_model_no_group_by() {
        let m = model_with_event_time(
            "eventstream",
            "SELECT event_date, user_id, amount FROM smelt.upstream WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "per-row model (no GROUP BY) must not error; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "per-row model (no GROUP BY) must be detected as incremental"
        );
    }

    /// Negative companion: a per-row SELECT whose SELECT list does **not**
    /// contain the `partition_column` alias must still return an error.
    #[test]
    fn test_detect_per_row_model_missing_partition_column() {
        let m = model_with_event_time(
            "eventstream_bad",
            "SELECT user_id, amount FROM smelt.upstream WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_err(),
            "per-row model missing partition_column in SELECT must error; got: {:?}",
            result
        );
        assert!(
            result.unwrap_err().contains("not found as alias"),
            "error must name the missing partition_column alias"
        );
    }

    #[test]
    fn test_detect_with_valid_unique_key() {
        let m = ModelInfo {
            name: "daily".to_string(),
            sql: "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec!["event_date".to_string(), "user_id".to_string()],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_detect_rejects_invalid_unique_key() {
        let m = ModelInfo {
            name: "daily".to_string(),
            sql: "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec!["nonexistent_col".to_string()],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("unique_key column 'nonexistent_col' not found"));
    }

    // --- BatchSafety tests ---

    #[test]
    fn test_batch_safety_simple_aggregate() {
        // Pure GROUP BY with no window functions → fully batch safe
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let safety = analyze_batch_safety(&m);
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }

    #[test]
    fn test_batch_safety_lag_function() {
        // LAG(col, 3) → bounded lookback of 3 periods
        let m = model_with_overrides(
            "lagged",
            "SELECT user_id, event_timestamp, LAG(amount, 3) OVER (ORDER BY event_timestamp) as prev FROM events",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let safety = analyze_batch_safety(&m);
        match safety {
            BatchSafety::BoundedSafe { context_days, .. } => {
                assert!(
                    context_days >= 3,
                    "expected context >= 3, got {}",
                    context_days
                );
            }
            other => panic!("Expected BoundedSafe, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_safety_unbounded_window() {
        // UNBOUNDED PRECEDING with RANGE → per partition only
        let m = model_with_overrides(
            "running",
            "SELECT user_id, event_timestamp, SUM(amount) OVER (ORDER BY event_timestamp RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) as running FROM events",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let safety = analyze_batch_safety(&m);
        match safety {
            BatchSafety::PerPartitionOnly { reason } => {
                assert!(reason.contains("unbounded"), "reason: {}", reason);
            }
            other => panic!("Expected PerPartitionOnly, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_safety_no_incremental_config() {
        // No incremental config → still fully batch safe (uses default period_days=1)
        let m = ModelInfo {
            name: "plain".to_string(),
            sql: "SELECT a, SUM(b) FROM t GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: None,
            incremental_config: None,
        };
        let safety = analyze_batch_safety(&m);
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }

    // --- batch_safety_from_bounds: the F1 BoundResult-map roll-up (BL1) ---

    /// All sources `Bounded(_, 0, 0)` → `FullyBatchSafe`.
    #[test]
    fn test_batch_safety_from_bounds_all_zero_margin_is_fully_safe() {
        let mut bounds = HashMap::new();
        bounds.insert(
            "orders".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            },
        );
        bounds.insert(
            "returns".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            },
        );
        let safety = batch_safety_from_bounds(&bounds).expect("must classify, not refuse");
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }

    /// An empty bound map (no timeseries sources at all) is also
    /// `FullyBatchSafe` — there is nothing to bound.
    #[test]
    fn test_batch_safety_from_bounds_empty_map_is_fully_safe() {
        let bounds: HashMap<String, BoundResult> = HashMap::new();
        assert_eq!(
            batch_safety_from_bounds(&bounds).expect("must classify"),
            BatchSafety::FullyBatchSafe
        );
    }

    /// Any source `Bounded` with `before + after > 0` classifies as
    /// `BoundedSafe(n)`, with `n` the *maximum* `before + after` across every
    /// bounded source — not the max of `before` and `after` taken
    /// separately.
    #[test]
    fn test_batch_safety_from_bounds_nonzero_margin_is_bounded_safe_with_max_n() {
        let mut bounds = HashMap::new();
        bounds.insert(
            "orders".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::days(2),
                after: crate::analysis::source_bounds::Seconds::ZERO,
            },
        );
        bounds.insert(
            "returns".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::days(1),
                after: crate::analysis::source_bounds::Seconds::days(1),
            },
        );
        let safety = batch_safety_from_bounds(&bounds).expect("must classify, not refuse");
        match safety {
            BatchSafety::BoundedSafe { context_days, .. } => {
                // max(before+after) = max(2 days, 2 days) = 2 days.
                assert_eq!(
                    context_days, 2,
                    "n must be max(before+after), not max(before, after)"
                );
            }
            other => panic!("Expected BoundedSafe, got {:?}", other),
        }
    }

    /// Any source `Unbounded` classifies the whole model `PerPartitionOnly`,
    /// naming the offending source.
    #[test]
    fn test_batch_safety_from_bounds_unbounded_source_is_per_partition_only() {
        let mut bounds = HashMap::new();
        bounds.insert(
            "orders".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            },
        );
        bounds.insert("running_balance".to_string(), BoundResult::Unbounded);
        let safety = batch_safety_from_bounds(&bounds).expect("must classify, not refuse");
        match safety {
            BatchSafety::PerPartitionOnly { reason } => {
                assert!(
                    reason.contains("running_balance"),
                    "reason must name the unbounded source: {reason}"
                );
            }
            other => panic!("Expected PerPartitionOnly, got {:?}", other),
        }
    }

    /// Fail-closed reject: any `NotDerivable` source refuses the whole model
    /// (`BatchedNotSafe`), naming the offending source — never silently
    /// downgraded to a coarser class computed from the other sources.
    #[test]
    fn test_batch_safety_from_bounds_not_derivable_refuses_naming_source() {
        let mut bounds = HashMap::new();
        bounds.insert(
            "orders".to_string(),
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            },
        );
        bounds.insert("bare_lag_source".to_string(), BoundResult::NotDerivable);
        let result = batch_safety_from_bounds(&bounds);
        assert!(
            result.is_err(),
            "NotDerivable source must refuse, not classify"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("bare_lag_source"),
            "error must name the offending source: {err}"
        );
    }

    // --- Partition-aligned OVER admissibility tests (Phase 3) ---

    /// A window function OVER (PARTITION BY device_id, session_seq) on a model
    /// whose partition_column is `device_id` should be admissible (superset rule).
    /// `{device_id, session_seq} ⊇ {device_id}` → admitted.
    #[test]
    fn test_admissible_over_partition_by_superset() {
        // partition_column = device_id; OVER uses (device_id, session_seq) — superset
        // Per-row model (no GROUP BY) so we just need partition_col in SELECT.
        let m = model_with_event_time(
            "sessions",
            "SELECT device_id, session_seq, event_date, \
             FIRST_VALUE(event_date) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS session_start_date \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "device_id",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "partition-aligned OVER (superset) must not be rejected; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "partition-aligned OVER (superset) model must classify as incremental"
        );
    }

    /// A window function OVER (PARTITION BY user_id) on a model whose
    /// partition_column is `device_id` should be rejected — the PARTITION BY
    /// keys `{user_id}` do not contain `device_id`.
    #[test]
    fn test_inadmissible_over_partition_by_disjoint() {
        // partition_column = device_id; OVER uses (user_id) — disjoint
        let m = model_with_event_time(
            "wrong_window",
            "SELECT device_id, event_date, \
             ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_ts) AS rn \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "device_id",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_err(),
            "OVER with disjoint PARTITION BY must be rejected; got: {:?}",
            result
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("window function"),
            "error must mention window function; got: {}",
            err
        );
    }

    // --- B0: unified pushdown-depth walk (derive_model_source_bounds) ---

    /// The confirmed §3.2 under-read harness: a model with a genuine bounded
    /// lookback (`RANGE BETWEEN INTERVAL '2 days' PRECEDING`) derives its
    /// write-window margin and its per-source scan margin from the *same*
    /// walk — there is only one `derive_model_bounds` call feeding both, so
    /// the two numbers cannot independently disagree. The derived `(before,
    /// after)` must match the frame's `INTERVAL '2 days'`, and the injection
    /// point must be `OuterClamp` (a real lookback keeps both layers).
    #[test]
    fn test_unified_walk_matches_frame_interval() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "windowed".to_string(),
            sql: "SELECT device_id, event_date, \
                  SUM(amount) OVER (PARTITION BY device_id ORDER BY event_ts \
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW) AS running \
                  FROM upstream"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        let (injection_point, bound) = bounds.get("upstream").expect("upstream must have a bound");
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(
                    *before,
                    crate::analysis::source_bounds::Seconds::days(2),
                    "derived before-margin must match the frame's INTERVAL '2 days'"
                );
                assert_eq!(*after, crate::analysis::source_bounds::Seconds::ZERO);
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
        assert_eq!(
            *injection_point,
            InjectionPoint::OuterClamp,
            "a genuine lookback margin must keep the outer clamp layer"
        );
    }

    /// A transparent single-source subquery (no lookback: no INTERVAL,
    /// no window function) derives a zero-margin bound and classifies as
    /// `InjectionPoint::Source` — the source-level filter alone is both the
    /// scan-pruning filter and the exact output clamp; no outer wrap.
    #[test]
    fn test_transparent_single_source_pushes_to_source_no_outer_wrap() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "passthrough".to_string(),
            sql: "SELECT event_date, user_id, amount FROM upstream WHERE amount > 0".to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        assert_eq!(bounds.len(), 1, "exactly one source-level filter expected");
        let (injection_point, bound) = bounds.get("upstream").expect("upstream must have a bound");
        assert_eq!(
            *bound,
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            }
        );
        assert_eq!(
            *injection_point,
            InjectionPoint::Source,
            "transparent slice must push to the source with no outer wrap"
        );
    }

    /// Fail-closed (unchanged by B0): a `NotDerivable` source (bare LAG/LEAD
    /// with no RANGE frame) still refuses at planning time, naming the
    /// construct.
    #[test]
    fn test_not_derivable_still_refuses() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "bare_lag".to_string(),
            sql: "SELECT device_id, event_date, LAG(amount) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev \
                  FROM upstream"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides {
                    allow_window_functions: true,
                    ..Default::default()
                },
            }),
        };

        let result = derive_model_source_bounds(&m, &graph);
        assert!(result.is_err(), "bare LAG without RANGE must refuse");
        let err = result.unwrap_err();
        assert!(
            err.contains("cannot derive a temporal bound"),
            "error must name the derivation failure: {}",
            err
        );
    }

    /// A window function OVER (PARTITION BY event_date) on a model whose
    /// partition_column is `event_date` should be admissible — equality is a
    /// superset too: `{event_date} ⊇ {event_date}`.
    #[test]
    fn test_admissible_over_partition_by_equals() {
        // partition_column = event_date; OVER uses (event_date) — exact equality
        let m = model_with_event_time(
            "daily_windowed",
            "SELECT event_date, user_id, \
             FIRST_VALUE(user_id) OVER (PARTITION BY event_date ORDER BY event_ts) AS first_user \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "partition-aligned OVER (equality) must not be rejected; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "partition-aligned OVER (equality) model must classify as incremental"
        );
    }

    // --- B1: UNION branch tracing ---

    fn upstream_ts_model(name: &str, event_time_column: &str, partition_column: &str) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: event_time_column.to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: None,
        }
    }

    /// Every UNION ALL branch traces `Traceable` to its own named source —
    /// each branch's source gets its own pushdown entry.
    #[test]
    fn test_union_all_traceable_branches_each_push_to_named_source() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("web_events", "event_ts", "event_date"));
        graph.add_model(upstream_ts_model("mobile_events", "event_ts", "event_date"));

        let m = ModelInfo {
            name: "all_events".to_string(),
            sql: "SELECT event_date, user_id FROM smelt.web_events \
                  UNION ALL \
                  SELECT event_date, user_id FROM smelt.mobile_events"
                .to_string(),
            refs: vec!["web_events".to_string(), "mobile_events".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        assert!(
            bounds.contains_key("web_events"),
            "keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
        assert!(
            bounds.contains_key("mobile_events"),
            "keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
    }

    /// A `StaticSeed` UNION branch (NULL literal for the event-time column)
    /// is named and rejected.
    #[test]
    fn test_union_all_static_seed_branch_rejected_naming_branch() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("web_events", "event_ts", "event_date"));

        let m = ModelInfo {
            name: "all_events".to_string(),
            sql: "SELECT event_date, user_id FROM smelt.web_events \
                  UNION ALL \
                  SELECT NULL AS event_date, user_id FROM smelt.backfill_seed"
                .to_string(),
            refs: vec!["web_events".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let result = derive_model_source_bounds(&m, &graph);
        assert!(result.is_err(), "StaticSeed UNION branch must be rejected");
        let err = result.unwrap_err();
        assert!(
            err.contains("UNION branch 2"),
            "error must name the branch: {err}"
        );
        assert!(
            err.to_lowercase().contains("static"),
            "error must name the construct: {err}"
        );
    }

    /// A `NotTraceable` UNION branch (not `StaticSeed`) is the conservative
    /// "stay at the outer clamp" case: the union as a whole gets no
    /// per-source pushdown entries, but is not rejected outright.
    #[test]
    fn test_union_all_not_traceable_branch_stays_at_outer_clamp() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("web_events", "event_ts", "event_date"));

        let m = ModelInfo {
            name: "all_events".to_string(),
            sql:
                "SELECT event_date, user_id FROM smelt.web_events \
                  UNION ALL \
                  SELECT CAST(event_date AS VARCHAR) AS event_date, user_id FROM smelt.other_events"
                    .to_string(),
            refs: vec!["web_events".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("must not be rejected");
        assert!(
            bounds.is_empty(),
            "NotTraceable branch must yield no per-source pushdown entries; got: {:?}",
            bounds
        );
    }

    // --- B1: subquery/CTE body tracing ---

    /// A CTE body that projects the model's partition column as a direct,
    /// traceable image of an upstream timeseries source's partition column
    /// licenses pushdown to that real source.
    #[test]
    fn test_cte_body_traceable_pushes_to_named_source() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("upstream", "event_date", "event_date"));

        let m = ModelInfo {
            name: "cte_model".to_string(),
            sql: "WITH staged AS (SELECT event_date, user_id FROM upstream) \
                  SELECT event_date, user_id FROM staged"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides {
                    allow_subqueries: true,
                    ..Default::default()
                },
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        assert!(
            bounds.contains_key("upstream"),
            "keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
    }

    /// A CTE body that projects the partition column through a non-monotone
    /// cast (`CAST(... AS VARCHAR)`, per the blacklist) is `NotTraceable`.
    /// Unlike `StaticSeed`, `NotTraceable` is a no-op here — it neither
    /// narrows nor refuses — so the model still gets whatever bound the
    /// existing Form A/B textual walk derives for `upstream` (a transparent,
    /// zero-margin slice here, since the SQL has no RANGE/BETWEEN INTERVAL
    /// pattern). This preserves the pre-B1 "CTE bypass" behaviour for
    /// models the monotonicity trace cannot (yet) prove, rather than
    /// regressing them to a hard refusal (see `restrict_ctx_for_derived_tables`
    /// doc comment for the `sessions.sql` real-fixture case this protects).
    #[test]
    fn test_cte_body_not_traceable_is_a_no_op_not_a_refusal() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("upstream", "event_date", "event_date"));

        let m = ModelInfo {
            name: "cte_model".to_string(),
            sql: "WITH staged AS (SELECT CAST(event_date AS VARCHAR) AS event_date, user_id \
                  FROM upstream) \
                  SELECT event_date, user_id FROM staged"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph)
            .expect("NotTraceable CTE body must not refuse the model");
        assert!(
            bounds.contains_key("upstream"),
            "keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
    }

    // --- B1: join driving-fact resolution ---

    /// A fact ⋈ lookup join where exactly one input (`fact`) traces its
    /// partition column resolves as the driving fact and gets the pushdown
    /// entry; the other (`lookup`) gets none — full-scanned, so the
    /// misfilter hazard of pushing a filter onto a non-driving input is
    /// zero by construction.
    #[test]
    fn test_join_exactly_one_traceable_input_is_driving_fact_others_full_scan() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("silver.fact", "event_ts", "event_date"));
        graph.add_model(upstream_ts_model(
            "silver.lookup",
            "updated_at",
            "snapshot_date",
        ));

        let m = ModelInfo {
            name: "joined".to_string(),
            sql: "SELECT f.event_date, f.user_id, g.attribute \
                  FROM smelt.silver.fact f \
                  JOIN smelt.silver.lookup g ON f.user_id = g.user_id"
                .to_string(),
            refs: vec!["silver.fact".to_string(), "silver.lookup".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        assert!(
            bounds.contains_key("silver.fact"),
            "driving fact must get a bound entry; keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
        assert!(
            !bounds.contains_key("silver.lookup"),
            "non-driving lookup input must be full-scanned (no entry); keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
    }

    // --- DC1: declared-monotonicity escape hatch (join driving-fact site) ---

    /// Without `assert_monotonic`, an opaque function wrapping the join
    /// driving fact's event-time column is `Undecidable` and the join
    /// driving-fact resolution refuses the model outright (fail-closed
    /// default — unchanged behaviour).
    #[test]
    fn test_declared_monotonic_absent_opaque_join_udf_is_refused() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("silver.fact", "event_ts", "event_date"));
        graph.add_model(upstream_ts_model(
            "silver.lookup",
            "updated_at",
            "snapshot_date",
        ));

        let m = ModelInfo {
            name: "joined".to_string(),
            sql: "SELECT my_custom_fn(f.event_date) AS event_date, f.user_id, g.attribute \
                  FROM smelt.silver.fact f \
                  JOIN smelt.silver.lookup g ON f.user_id = g.user_id"
                .to_string(),
            refs: vec!["silver.fact".to_string(), "silver.lookup".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let result = derive_model_source_bounds(&m, &graph);
        assert!(
            result.is_err(),
            "an opaque function over the join driving fact must be refused without the \
             declaration; got {result:?}"
        );
    }

    /// With `assert_monotonic: true`, the same opaque function widens the
    /// undecidable verdict — the join driving fact resolves and gets a bound
    /// entry, exactly as if the column had been projected bare.
    #[test]
    fn test_declared_monotonic_admits_opaque_join_udf() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("silver.fact", "event_ts", "event_date"));
        graph.add_model(upstream_ts_model(
            "silver.lookup",
            "updated_at",
            "snapshot_date",
        ));

        let m = ModelInfo {
            name: "joined".to_string(),
            sql: "SELECT my_custom_fn(f.event_date) AS event_date, f.user_id, g.attribute \
                  FROM smelt.silver.fact f \
                  JOIN smelt.silver.lookup g ON f.user_id = g.user_id"
                .to_string(),
            refs: vec!["silver.fact".to_string(), "silver.lookup".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: true,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph)
            .expect("declared monotonicity must admit the opaque-function driving fact");
        assert!(
            bounds.contains_key("silver.fact"),
            "driving fact must get a bound entry once declared monotonic; keys: {:?}",
            bounds.keys().collect::<Vec<_>>()
        );
    }

    /// The declaration must not rescue a *positively disproven* join driving
    /// fact — a row-nondeterministic function in the event-time position
    /// stays refused even with `assert_monotonic: true`.
    #[test]
    fn test_declared_monotonic_does_not_admit_row_nondeterministic_join_driving_fact() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model("silver.fact", "event_ts", "event_date"));
        graph.add_model(upstream_ts_model(
            "silver.lookup",
            "updated_at",
            "snapshot_date",
        ));

        let m = ModelInfo {
            name: "joined".to_string(),
            sql: "SELECT RANDOM() AS event_date, f.user_id, g.attribute \
                  FROM smelt.silver.fact f \
                  JOIN smelt.silver.lookup g ON f.user_id = g.user_id"
                .to_string(),
            refs: vec!["silver.fact".to_string(), "silver.lookup".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: true,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let result = derive_model_source_bounds(&m, &graph);
        assert!(
            result.is_err(),
            "RANDOM() declared monotonic must still be refused — a row-nondeterministic \
             value can never occupy the event-time position; got {result:?}"
        );
    }

    // --- B2: bounded-RANGE cross-partition windows + forward-reach walk ---

    /// The sessionization harness: `LAG(...) OVER (PARTITION BY device_id
    /// ORDER BY event_ts RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND
    /// CURRENT ROW)` is admitted (`find_inadmissible_over` returns `None`)
    /// even though the model's own `partition_column` is a *derived*
    /// `session_start_date` unrelated to the window's `PARTITION BY
    /// device_id` — the bounded RANGE frame is the licensing lookback. The
    /// same walk that admits it derives the upstream source's bound as
    /// `Bounded(_, 30min, 0)` via `derive_model_source_bounds`.
    #[test]
    fn test_bounded_range_lag_admitted_despite_derived_partition_column() {
        let sql = "LAG(event_ts) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS prev_ts";
        assert_eq!(
            find_inadmissible_over(sql, "session_start_date"),
            None,
            "a bounded RANGE INTERVAL frame must be admitted regardless of PARTITION BY alignment"
        );

        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model(
            "silver.events_parsed",
            "event_ts",
            "event_ts",
        ));

        let m = ModelInfo {
            name: "sessions".to_string(),
            sql: "SELECT device_id, session_start_date, \
                  LAG(event_ts) OVER (PARTITION BY device_id ORDER BY event_ts \
                  RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS prev_ts \
                  FROM smelt.silver.events_parsed"
                .to_string(),
            refs: vec!["silver.events_parsed".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "session_start_date".to_string(),
                partition_column: "session_start_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        let (_, bound) = bounds
            .get("silver.events_parsed")
            .expect("events_parsed must have a bound entry");
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(
                    *before,
                    crate::analysis::source_bounds::Seconds::minutes(30),
                    "the 30-minute RANGE frame must derive a 30-minute lookback"
                );
                assert_eq!(*after, crate::analysis::source_bounds::Seconds::ZERO);
            }
            other => panic!("Expected Bounded(_, 30min, 0), got {:?}", other),
        }
    }

    /// Fail-closed: a bare `LAG` with no `RANGE` frame at all stays
    /// inadmissible when `PARTITION BY` is not aligned to the partition
    /// column — the bounded-RANGE exception does not extend to frame-less
    /// windows.
    #[test]
    fn test_bare_lag_no_range_stays_inadmissible() {
        let sql = "LAG(event_ts) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev_ts";
        assert!(
            find_inadmissible_over(sql, "session_start_date").is_some(),
            "a bare LAG with no RANGE frame must stay inadmissible"
        );
    }

    /// Fail-closed: a `ROWS BETWEEN ... PRECEDING` frame does not qualify
    /// for the bounded-RANGE exception (`has_bounded_range_interval_frame`
    /// requires the `RANGE` keyword specifically) — a non-aligned
    /// `PARTITION BY` still refuses.
    #[test]
    fn test_rows_frame_does_not_get_range_exception() {
        let sql = "SUM(amount) OVER (PARTITION BY device_id ORDER BY event_ts \
                   ROWS BETWEEN 5 PRECEDING AND CURRENT ROW) AS running";
        assert!(
            find_inadmissible_over(sql, "session_start_date").is_some(),
            "a ROWS frame must not be admitted by the RANGE-INTERVAL exception"
        );
    }

    /// Fail-closed: a `GROUPS BETWEEN ... PRECEDING` frame likewise does not
    /// qualify for the bounded-RANGE exception.
    #[test]
    fn test_groups_frame_does_not_get_range_exception() {
        let sql = "SUM(amount) OVER (PARTITION BY device_id ORDER BY event_ts \
                   GROUPS BETWEEN 5 PRECEDING AND CURRENT ROW) AS running";
        assert!(
            find_inadmissible_over(sql, "session_start_date").is_some(),
            "a GROUPS frame must not be admitted by the RANGE-INTERVAL exception"
        );
    }

    /// Fail-closed: `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND UNBOUNDED
    /// FOLLOWING` is not admitted by the bounded-RANGE exception — the
    /// forward reach is unbounded even though the backward reach is finite.
    #[test]
    fn test_unbounded_following_range_stays_inadmissible() {
        let sql = "SUM(amount) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND UNBOUNDED FOLLOWING) AS running";
        assert!(
            find_inadmissible_over(sql, "session_start_date").is_some(),
            "UNBOUNDED FOLLOWING must not be admitted even with a bounded PRECEDING half"
        );
    }

    /// The forward-reach (`after_secs`) walk shares B0's single derivation:
    /// a model whose only temporal pattern is a `RANGE ... FOLLOWING` frame
    /// derives a nonzero `after` and a zero `before` from `derive_model_source_bounds`,
    /// and the injection point is `OuterClamp` (a genuine forward margin needs
    /// both the widened source scan and the exact output clamp).
    #[test]
    fn test_forward_reach_derives_outer_clamp_injection_point() {
        let mut graph = ModelGraph::new();
        graph.add_model(upstream_ts_model(
            "silver.events_parsed",
            "event_ts",
            "event_ts",
        ));

        let m = ModelInfo {
            name: "attribution".to_string(),
            sql: "SELECT device_id, event_date, \
                  LEAD(event_ts) OVER (PARTITION BY device_id ORDER BY event_ts \
                  RANGE BETWEEN CURRENT ROW AND INTERVAL '2 hours' FOLLOWING) AS next_ts \
                  FROM smelt.silver.events_parsed"
                .to_string(),
            refs: vec!["silver.events_parsed".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        let (injection_point, bound) = bounds
            .get("silver.events_parsed")
            .expect("events_parsed must have a bound entry");
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(*before, crate::analysis::source_bounds::Seconds::ZERO);
                assert_eq!(*after, crate::analysis::source_bounds::Seconds::hours(2));
            }
            other => panic!("Expected Bounded(_, 0, 2h), got {:?}", other),
        }
        assert_eq!(
            *injection_point,
            InjectionPoint::OuterClamp,
            "a genuine forward margin must keep the outer clamp layer"
        );
    }
}
