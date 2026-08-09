//! Classifier for the `refresh: keyed` mode.
//!
//! See `docs/specs/incremental_models.md` §"The key grain (`grain: key`)" for the normative spec.
//! This module classifies the direct-monoid families (additive fold,
//! extremal/lattice fold), the order-monotone overwrite family
//! (`MAX_BY`/`MIN_BY`, `docs/plans/20260809-keyed-frontier.md` Phase 1),
//! and the plain-overwrite family (`ANY_VALUE`, Phase 3) — across both
//! derived run shapes (window-forward / snapshot-reconcile, §"The two run
//! shapes"). The once-write family (`COALESCE`) is not yet classified here.
//!
//! The classifier is a pure function that reads an inlined SELECT
//! (post function expansion) plus a small source-timeseries lookup
//! and derives:
//!
//! - `unique_key` — the GROUP BY column list.
//! - `aggregator_columns` — per non-key projection, the
//!   `(per_partition_agg, cross_partition_combiner)` pair from a
//!   fixed allowlist.
//! - `driving_source` — the single source the rule iterates over,
//!   clocked (window-forward) or unclocked (snapshot-reconcile) — the run
//!   shape is derived from which.
//!
//! Returns a `CumulativeClassification` on success or a list of
//! `KeyedDiagnostic`s on rejection.

use serde::Serialize;
use smelt_core::config::TimeseriesConfig;
use std::collections::HashMap;

use crate::analysis::monotonicity::NONDETERMINISTIC_FUNCTIONS;
use crate::analysis::source_bounds::{resolve_single_anchor, AnchorAmbiguity};
use crate::analysis::{analyze_select, SelectItemKind};
use smelt_types::SqlFunction;

/// A per-partition aggregator paired with its cross-partition combiner.
///
/// The combiner is a fixed lookup off the per-partition aggregator —
/// `COUNT → SUM`, everything else folds onto itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregatorColumn {
    /// The output column name (the projection's `AS` alias).
    pub output_name: String,
    /// The SQL aggregate function called on the source rows for one partition.
    pub per_partition_agg: String,
    /// The SQL aggregate function that combines the target's value with
    /// the new partition's value. Stored as a name; the rule renders it
    /// as `f(target.col, delta.col)` or the equivalent (e.g. `LEAST`/`GREATEST`).
    pub cross_partition_combiner: CrossPartitionCombiner,
}

/// How target and delta values combine for a column on `MERGE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CrossPartitionCombiner {
    /// `target.c + delta.c` — additive.
    Sum,
    /// `LEAST(target.c, delta.c)` — minimum.
    Min,
    /// `GREATEST(target.c, delta.c)` — maximum.
    Max,
    /// `target.c AND delta.c`.
    BoolAnd,
    /// `target.c OR delta.c`.
    BoolOr,
    /// `target.c & delta.c`.
    BitAnd,
    /// `target.c | delta.c`.
    BitOr,
    /// `xor(target.c, delta.c)` — function form is DuckDB-compatible and works
    /// in Postgres as well; the `#` infix operator is Postgres-only.
    BitXor,
    /// The order-monotone overwrite family (`MAX_BY`/`MIN_BY`,
    /// `incremental_models.md` §"The column-family catalogue"): the delta's
    /// value wins iff its ordering value strictly beats the target's stored
    /// ordering value — incumbent wins on a tie (§"Ordering ties"). Storage
    /// decision (Phase 1, `docs/plans/20260809-keyed-frontier.md`): the
    /// ordering value is never a hidden shadow column — `ordering_column` is
    /// the output name of another column in the *same* projection that
    /// tracks the running `MAX`/`MIN` of the ordering expression (a plain
    /// extremal-fold `AggregatorColumn`), so target/delta values for the
    /// comparison always come from that column's own `target.<name>` /
    /// `delta.<name>` refs — no decomposed/hidden state
    /// (`docs/specs/incremental_models.md` §Known Divergences).
    OrderMonotone { ordering_column: String },
    /// The plain-overwrite family (`ANY_VALUE(...)`,
    /// `incremental_models.md` §"The column-family catalogue"): the delta's
    /// value always wins — the incoming row is the current observation, no
    /// target comparison is made. Admitted only under the snapshot-reconcile
    /// run shape (Phase 3, `docs/plans/20260809-keyed-frontier.md`); refused
    /// window-forward (`KeyedUnknownCombiner`).
    PlainOverwrite,
}

impl CrossPartitionCombiner {
    /// Render the SQL expression that produces the merged value for one column.
    pub fn render(&self, target_col: &str, delta_col: &str) -> String {
        match self {
            CrossPartitionCombiner::Sum => format!("{} + {}", target_col, delta_col),
            CrossPartitionCombiner::Min => format!("LEAST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::Max => format!("GREATEST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::BoolAnd => format!("{} AND {}", target_col, delta_col),
            CrossPartitionCombiner::BoolOr => format!("{} OR {}", target_col, delta_col),
            CrossPartitionCombiner::BitAnd => format!("{} & {}", target_col, delta_col),
            CrossPartitionCombiner::BitOr => format!("{} | {}", target_col, delta_col),
            CrossPartitionCombiner::BitXor => format!("xor({}, {})", target_col, delta_col),
            CrossPartitionCombiner::OrderMonotone { ordering_column } => {
                // `target_col`/`delta_col` are already qualified
                // (`target.<name>`/`delta.<name>`, the caller's own
                // convention, `smelt-runtime::cumulative::build_cumulative_
                // merge_sql`) — the ordering column lives in the same
                // target/delta scope, so the same qualifiers apply.
                let target_qualifier = target_col.rsplit_once('.').map_or("target", |(q, _)| q);
                let delta_qualifier = delta_col.rsplit_once('.').map_or("delta", |(q, _)| q);
                let target_ord = format!("{target_qualifier}.{ordering_column}");
                let delta_ord = format!("{delta_qualifier}.{ordering_column}");
                // Strict `>` — incumbent wins on a tie (§"Ordering ties":
                // "the delta wins iff `delta.ordering > target.ordering`
                // (strict); on equality the incumbent wins").
                format!(
                    "CASE WHEN {delta_ord} > {target_ord} THEN {delta_col} ELSE {target_col} END"
                )
            }
            CrossPartitionCombiner::PlainOverwrite => delta_col.to_string(),
        }
    }
}

/// Lookup a per-partition aggregator name and return its cross-partition
/// combiner. The gate is the shared algebraic-discriminants classifier
/// (`analysis::discriminants::combiner_discriminants`) — only a monoid
/// combiner is admitted; `None` means the aggregator is not a monoid (either
/// holistic, e.g. `AVG`/`STRING_AGG`, or unrecognised).
pub fn combiner_for(agg_name: &str) -> Option<CrossPartitionCombiner> {
    let function = SqlFunction::from_name(agg_name)?;
    let discriminants = crate::analysis::discriminants::combiner_discriminants(function, false);
    if !discriminants.is_monoid {
        return None;
    }
    match function {
        SqlFunction::Count | SqlFunction::Sum => Some(CrossPartitionCombiner::Sum),
        SqlFunction::Min => Some(CrossPartitionCombiner::Min),
        SqlFunction::Max => Some(CrossPartitionCombiner::Max),
        SqlFunction::BoolAnd => Some(CrossPartitionCombiner::BoolAnd),
        SqlFunction::BoolOr => Some(CrossPartitionCombiner::BoolOr),
        SqlFunction::BitAnd => Some(CrossPartitionCombiner::BitAnd),
        SqlFunction::BitOr => Some(CrossPartitionCombiner::BitOr),
        SqlFunction::BitXor => Some(CrossPartitionCombiner::BitXor),
        _ => None,
    }
}

/// A diagnostic code emitted by the keyed classifier.
///
/// Mirrors `incremental_models.md` §"Key-grain diagnostic codes".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum KeyedDiagnostic {
    KeyedRequiresGroupBy,
    KeyedUnknownCombiner {
        projection: String,
        offending: String,
    },
    KeyedGroupByContainsPartitionColumn {
        partition_column: String,
    },
    KeyedForbidsWindowFunctions,
    KeyedForbidsNondeterministic {
        offending: String,
    },
    /// No clocked driving source was found, AND no single unambiguous
    /// source could be resolved to derive the snapshot-reconcile run shape
    /// either — e.g. the FROM clause joins more than one candidate source
    /// with none of them clocked. Genuinely unsupportable, not a "not yet"
    /// refusal (`docs/specs/incremental_models.md` §"The two run shapes").
    KeyedSnapshotPostureUnsupported,
    /// A fold-family projection (additive, extremal/lattice, or
    /// order-monotone overwrite) under the snapshot-reconcile run shape
    /// (`docs/specs/incremental_models.md` §"Admission matrix"): these
    /// families consume events, not observations — re-folding a mutable
    /// snapshot double-counts (additive) or computes a history observation
    /// instead of the current value (observer semantics, the other
    /// families).
    KeyedSnapshotSourceUnsupportedColumn {
        projection: String,
        family: String,
        reason: String,
    },
    KeyedMultipleDrivingSources {
        candidates: Vec<String>,
    },
    KeyedSqlNotParseable,
}

impl std::fmt::Display for KeyedDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyedDiagnostic::KeyedRequiresGroupBy => write!(
                f,
                "KeyedRequiresGroupBy: refresh: keyed SELECT must have a GROUP BY \
                 clause — the GROUP BY columns are the unique key"
            ),
            KeyedDiagnostic::KeyedUnknownCombiner {
                projection,
                offending,
            } => write!(
                f,
                "KeyedUnknownCombiner: projection `{}` uses `{}`, which is not in the \
                 keyed aggregator allowlist (COUNT, SUM, MIN, MAX, BOOL_AND, BOOL_OR, \
                 BIT_AND, BIT_OR, BIT_XOR). Composite expressions over aggregates are not \
                 allowed — split into separate projections.",
                projection, offending
            ),
            KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { partition_column } => {
                write!(
                    f,
                    "KeyedGroupByContainsPartitionColumn: the GROUP BY contains the driving \
                     source's partition_column `{}`, which produces a per-partition output shape, \
                     not the keyed one — switch to `refresh: batched` + `timeseries:` instead, or \
                     declare `timeseries:` on this model to stay keyed",
                    partition_column
                )
            }
            KeyedDiagnostic::KeyedForbidsWindowFunctions => write!(
                f,
                "KeyedForbidsWindowFunctions: window functions (OVER (...)) are not allowed \
                 in refresh: keyed SELECTs — the keyed state is the window"
            ),
            KeyedDiagnostic::KeyedForbidsNondeterministic { offending } => write!(
                f,
                "KeyedForbidsNondeterministic: non-deterministic function `{}` is not \
                 allowed in refresh: keyed SELECTs — cross-window combine requires \
                 deterministic per-window output",
                offending
            ),
            KeyedDiagnostic::KeyedSnapshotPostureUnsupported => write!(
                f,
                "KeyedSnapshotPostureUnsupported: this model has no clocked driving source \
                 (no timeseries-tagged source in the FROM clause), and no single unambiguous \
                 source could be resolved to derive the snapshot-reconcile run shape either \
                 — the FROM clause must join exactly one candidate source when none is \
                 clocked. Declare `timeseries:` on a driving source to use the window-forward \
                 run shape instead, or reduce the FROM clause to a single source."
            ),
            KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                projection,
                family,
                reason,
            } => write!(
                f,
                "KeyedSnapshotSourceUnsupportedColumn: projection `{projection}` is a \
                 {family} column, which is refused under the snapshot-reconcile run shape \
                 (no clocked driving source) — {reason}. Wrap it as `ANY_VALUE(...)` for the \
                 plain-overwrite family instead, or declare `timeseries:` on a driving source \
                 to use the window-forward run shape."
            ),
            KeyedDiagnostic::KeyedMultipleDrivingSources { candidates } => write!(
                f,
                "KeyedMultipleDrivingSources: multiple timeseries-tagged sources in the \
                 FROM clause ({}). v1 supports exactly one driving source.",
                candidates.join(", ")
            ),
            KeyedDiagnostic::KeyedSqlNotParseable => {
                write!(f, "SQL body could not be parsed for keyed classification")
            }
        }
    }
}

/// The result of classifying a `cumulative_aggregate` model.
#[derive(Debug, Clone, Serialize)]
pub struct CumulativeClassification {
    /// Columns from the GROUP BY list. The order matches the SELECT's
    /// GROUP BY ordering.
    pub unique_key: Vec<String>,
    /// Non-key projections with their derived combiners.
    pub aggregator_columns: Vec<AggregatorColumn>,
    /// The single source the rule iterates over. `name` is the model/source
    /// name as it appears in `smelt.<path>` references; `timeseries` is
    /// `Some` (window-forward) or `None` (snapshot-reconcile) — see
    /// [`DrivingSource::timeseries`].
    pub driving_source: DrivingSource,
}

impl CumulativeClassification {
    /// Whether this classification derived the snapshot-reconcile run shape
    /// (`docs/specs/incremental_models.md` §"The two run shapes") — zero
    /// clocked sources in the FROM clause. Derived from the classifier's own
    /// resolved [`DrivingSource`], never a second independent check.
    pub fn is_snapshot_reconcile(&self) -> bool {
        self.driving_source.timeseries.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DrivingSource {
    pub name: String,
    /// `Some` for the window-forward run shape (the source's own declared
    /// `timeseries:` block, the driving-fact/anchor proof); `None` for the
    /// snapshot-reconcile run shape — zero clocked sources in the FROM
    /// clause (`docs/specs/incremental_models.md` §"The two run shapes").
    /// The run shape is derived from this field, never declared.
    pub timeseries: Option<TimeseriesConfig>,
}

/// Lookup table for a source's `timeseries:` declaration. The classifier
/// uses this to identify the driving source and to enforce
/// `KeyedGroupByContainsPartitionColumn`.
pub type SourceTimeseriesMap = HashMap<String, TimeseriesConfig>;

/// Derive a `grain: key` model's `unique_key` from its own GROUP BY: the
/// SELECT aliases corresponding to each GROUP BY expression, falling back
/// to the raw expression text for a GROUP BY expression with no matching
/// non-aggregate projection (e.g. an expression-based GROUP BY).
fn group_by_unique_key_from_analysis(analysis: &crate::analysis::SelectAnalysis) -> Vec<String> {
    let mut unique_key: Vec<String> = Vec::new();
    for group_expr in &analysis.group_by_exprs {
        let matched = analysis.items.iter().find_map(|item| match item {
            SelectItemKind::GroupByKey { text, alias, .. } if text == group_expr => {
                Some(alias.clone())
            }
            _ => None,
        });
        if let Some(alias) = matched {
            unique_key.push(alias);
        } else {
            unique_key.push(group_expr.clone());
        }
    }
    unique_key
}

/// Derive a `grain: key` model's `unique_key` from its own GROUP BY clause,
/// independent of the full [`classify_cumulative`] admission (aggregator
/// allowlisting, driving-source resolution). Plan derivation
/// (`smelt-db::queries::maintenance::derive_model_maintenance_plan`) needs
/// only the key itself — admitted or not — to thread the model's real
/// `unique_key` into `PlanGrain::Key`/`SourceFacts` instead of a hardcoded
/// empty vec.
///
/// Returns an empty vec when the SQL doesn't parse or declares no GROUP BY.
pub fn group_by_unique_key(sql: &str) -> Vec<String> {
    match analyze_select(sql) {
        Some(analysis) => group_by_unique_key_from_analysis(&analysis),
        None => Vec::new(),
    }
}

/// Compare a **declared** top-level `unique_key:` (`docs/specs/models.md`
/// §"Refresh axis") against the GROUP-BY-derived key [`group_by_unique_key`]
/// computes for the same SQL — a leaf classifier check over one
/// already-bounded model's own text (`architecture.md` §"Property
/// composition walk rule"), order-independent (declaring the same columns in
/// a different order is agreement, not a mismatch).
///
/// `Ok(())` on agreement (including when `declared` is empty and so is the
/// derived key). `Err((declared, derived))` on disagreement — both lists, in
/// their original declared/derived order, for the caller to name in a
/// diagnostic (`models.md` §"Constraint violations": "For aggregated key
/// bodies: `unique_key` ≠ the `GROUP BY` column set → hard error (checked
/// restatement)"). This function only compares; it does not decide which
/// list wins — `smelt-db::queries::maintenance` is the sole caller that
/// turns a mismatch into a refused plan.
pub fn declared_unique_key_matches(
    declared: &[String],
    sql: &str,
) -> Result<(), (Vec<String>, Vec<String>)> {
    let derived = group_by_unique_key(sql);
    let mut declared_sorted = declared.to_vec();
    declared_sorted.sort();
    let mut derived_sorted = derived.clone();
    derived_sorted.sort();
    if declared_sorted == derived_sorted {
        Ok(())
    } else {
        Err((declared.to_vec(), derived))
    }
}

/// Classify a `cumulative_aggregate` model.
///
/// `sql` is the inlined model SQL (post function expansion). `refs` is the
/// list of `smelt.<path>` references discovered in the FROM clause. Source
/// timeseries declarations are looked up via `source_timeseries`.
/// `model_has_timeseries` is whether the model's own frontmatter declares a
/// `timeseries:` block — it narrows `KeyedGroupByContainsPartitionColumn`
/// to the no-`timeseries:` case (a model that declares its own
/// `timeseries:` is instead decided by the key-temporal-locality gate,
/// `maintenance::locality::establish_locality`).
///
/// Returns the classification on success, or a vector of diagnostics
/// describing every classifier rejection (the function does not short-circuit
/// on the first error — it surfaces every problem it can detect).
pub fn classify_cumulative(
    sql: &str,
    refs: &[String],
    source_timeseries: &SourceTimeseriesMap,
    model_has_timeseries: bool,
) -> Result<CumulativeClassification, Vec<KeyedDiagnostic>> {
    let mut diagnostics = Vec::new();

    let analysis = match analyze_select(sql) {
        Some(a) => a,
        None => {
            return Err(vec![KeyedDiagnostic::KeyedSqlNotParseable]);
        }
    };

    // Rule: GROUP BY required.
    if analysis.group_by_exprs.is_empty() {
        diagnostics.push(KeyedDiagnostic::KeyedRequiresGroupBy);
    }

    // Build the unique_key as the SELECT aliases corresponding to GROUP BY
    // expressions. Each GROUP BY expression is matched to a projection by
    // textual identity (the analyser already resolves ordinals).
    let unique_key = group_by_unique_key_from_analysis(&analysis);

    // Find the driving source and derive the run shape (`docs/specs/
    // incremental_models.md` §"The two run shapes") BEFORE walking the
    // projection list — every column family's admission depends on which
    // run shape the model derives (the admission matrix is a `(column
    // family × run shape)` table), so the run shape must be known first.
    //
    // Exactly one clocked (timeseries-tagged) source in the FROM clause is
    // the window-forward driving source — the shared anchor resolver
    // (`resolve_single_anchor`) also used by `resolve_join_driving_fact`'s
    // alias-scoped monotonicity trace. Zero clocked sources derives
    // snapshot-reconcile instead of refusing outright: a single
    // unambiguous joined source (of ANY posture — the FROM clause need not
    // register a `timeseries:` block for this resolution) stands in as
    // "the source" the whole-scan reconciliation re-scans. Two or more
    // clocked sources stays `KeyedMultipleDrivingSources`.
    let alias_sources: Vec<(String, String)> =
        smelt_parser::File::cast(smelt_parser::parse(sql).syntax())
            .and_then(|file| file.select_stmt())
            .and_then(|select| select.from_clause())
            .map(|from_clause| {
                crate::analysis::source_bounds::from_clause_alias_sources(&from_clause)
            })
            .unwrap_or_default();

    let clocked_resolution = resolve_single_anchor(&alias_sources, |source_name| {
        let key = format!("smelt.{source_name}");
        if !refs.iter().any(|r| r == &key) {
            return None;
        }
        source_timeseries.get(&key).map(|ts| DrivingSource {
            name: key.clone(),
            timeseries: Some(ts.clone()),
        })
    });

    let (driving_source, is_snapshot_reconcile) = match clocked_resolution {
        Ok(ds) => (Some(ds), false),
        Err(AnchorAmbiguity::Multiple(candidates)) => {
            diagnostics.push(KeyedDiagnostic::KeyedMultipleDrivingSources {
                candidates: candidates
                    .into_iter()
                    .map(|n| format!("smelt.{n}"))
                    .collect(),
            });
            (None, false)
        }
        Err(AnchorAmbiguity::NoCandidate) => {
            let snapshot_source = resolve_single_anchor(&alias_sources, |source_name| {
                let key = format!("smelt.{source_name}");
                refs.iter().any(|r| r == &key).then_some(key)
            });
            match snapshot_source {
                Ok(name) => (
                    Some(DrivingSource {
                        name,
                        timeseries: None,
                    }),
                    true,
                ),
                Err(_) => {
                    diagnostics.push(KeyedDiagnostic::KeyedSnapshotPostureUnsupported);
                    (None, false)
                }
            }
        }
    };

    // Walk the projection list. Non-key projections must be allowlisted
    // aggregator calls (per the admission matrix, gated on `is_snapshot_
    // reconcile` above). GroupByKey items are the key columns.
    let mut aggregator_columns: Vec<AggregatorColumn> = Vec::new();
    for item in &analysis.items {
        match item {
            SelectItemKind::GroupByKey { text, alias, .. } => {
                // A "GroupByKey" item is the analyser's classification for
                // any non-aggregate expression. If the projection's text
                // appears in the GROUP BY, it is genuinely a key column.
                // Otherwise — whether a composite expression (`SUM(x) + 1`)
                // or a bare column reference — it is not a valid keyed
                // projection either way: the catalogue's overwrite families
                // are only ever expressed as aggregate calls (`MAX_BY(...)`,
                // `ANY_VALUE(...)`), never a bare passthrough column
                // (`docs/specs/incremental_models.md` §"The column-family
                // catalogue").
                let in_group_by = analysis.group_by_exprs.iter().any(|g| g == text);
                if !in_group_by {
                    diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                        projection: alias.clone(),
                        offending: format!(
                            "composite expression `{}` — wrap it as `MAX_BY({}, <ordering>)` \
                             for the order-monotone overwrite family (window-forward), or \
                             `ANY_VALUE({})` for the plain-overwrite family \
                             (snapshot-reconcile only)",
                            text.trim(),
                            text.trim(),
                            text.trim()
                        ),
                    });
                }
            }
            SelectItemKind::CountDistinct { alias, .. } => {
                // COUNT(DISTINCT x) is not commutative under merge (the union
                // of distinct values across partitions cannot be reconstructed
                // from per-partition counts). Refuse.
                diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                    projection: alias.clone(),
                    offending: "COUNT(DISTINCT)".to_string(),
                });
            }
            SelectItemKind::OtherAggregate {
                text, alias, expr, ..
            } => {
                // Extract the outer function name from the already-parsed
                // expression (no string re-parse) and confirm it against the
                // typed aggregate classifier — the same
                // `SqlFunction::is_aggregate` predicate `analysis::mod` used
                // to classify this item as `OtherAggregate` in the first
                // place.
                let agg_name = expr
                    .as_function_call()
                    .and_then(|f| f.name())
                    .map(|n| n.to_ascii_uppercase())
                    .filter(|n| SqlFunction::from_name(n).is_some_and(|f| f.is_aggregate()));

                // The plain-overwrite family (`ANY_VALUE`,
                // `incremental_models.md` §"The column-family catalogue"):
                // admitted only under snapshot-reconcile; refused
                // window-forward naming the `MAX_BY` fix.
                if agg_name.as_deref() == Some("ANY_VALUE") {
                    if is_snapshot_reconcile {
                        if is_direct_function_call(text, "ANY_VALUE") {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: "ANY_VALUE".to_string(),
                                cross_partition_combiner: CrossPartitionCombiner::PlainOverwrite,
                            });
                        } else {
                            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                                projection: alias.clone(),
                                offending: format!("composite expression `{}`", text.trim()),
                            });
                        }
                    } else {
                        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                            projection: alias.clone(),
                            offending: format!(
                                "ANY_VALUE (the plain-overwrite family is snapshot-reconcile \
                                 only) — wrap the value as `MAX_BY({}, <ordering>)` for the \
                                 order-monotone overwrite family instead",
                                text.trim()
                            ),
                        });
                    }
                    continue;
                }

                // The order-monotone overwrite family (`MAX_BY`/`MIN_BY` —
                // `ArgMax`/`ArgMin`, `Monotone::Order`) is not gated through
                // `combiner_for`'s `is_monoid` allowlist (it is a semilattice
                // fold, not a commutative monoid) — handle it up front and
                // move to the next projection.
                let order_monotone = agg_name.as_deref().and_then(|n| {
                    let sql_fn = SqlFunction::from_name(n)?;
                    let is_order =
                        crate::analysis::discriminants::combiner_discriminants(sql_fn, false)
                            .monotone
                            == crate::analysis::discriminants::Monotone::Order;
                    is_order.then_some((n.to_string(), sql_fn))
                });
                if let Some((agg_upper, sql_fn)) = order_monotone {
                    if is_snapshot_reconcile {
                        diagnostics.push(KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                            projection: alias.clone(),
                            family: "order-monotone overwrite".to_string(),
                            reason: "observer semantics — MAX_BY/MIN_BY over successive \
                                     snapshots retains a stale incumbent forever if a mutation \
                                     regresses the ordering value"
                                .to_string(),
                        });
                    } else {
                        classify_order_monotone_column(
                            &analysis,
                            text,
                            alias,
                            expr,
                            sql_fn,
                            &agg_upper,
                            &mut aggregator_columns,
                            &mut diagnostics,
                        );
                    }
                    continue;
                }

                let combiner = agg_name.as_deref().and_then(combiner_for);
                match (agg_name, combiner) {
                    (Some(agg_upper), Some(combiner)) => {
                        if is_snapshot_reconcile {
                            let (family, reason) = snapshot_refusal_reason(&agg_upper);
                            diagnostics.push(
                                KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                                    projection: alias.clone(),
                                    family: family.to_string(),
                                    reason: reason.to_string(),
                                },
                            );
                            continue;
                        }
                        // Verify the projection is a *direct* call — no
                        // composition like `SUM(x) + 1`. We detect this by
                        // checking that the projection text starts with the
                        // function name and ends with `)`, ignoring whitespace.
                        if is_direct_function_call(text, &agg_upper) {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: agg_upper,
                                cross_partition_combiner: combiner,
                            });
                        } else {
                            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                                projection: alias.clone(),
                                offending: format!("composite expression `{}`", text.trim()),
                            });
                        }
                    }
                    (Some(name), None) => {
                        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                            projection: alias.clone(),
                            offending: name,
                        });
                    }
                    (None, _) => {
                        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                            projection: alias.clone(),
                            offending: text.clone(),
                        });
                    }
                }
            }
        }
    }

    // Rule: no window functions in the outer body.
    //
    // Known walk-invariant violation: this is a raw whole-SQL text scan in an
    // admission path, not a leaf classifier invoked by the shared
    // composition walk (`docs/specs/architecture.md` §"Property composition
    // walk rule"). It predates that walk and is excluded from the
    // `walk_coverage` structural gate (`crates/smelt-logical/tests/walk_coverage.rs`'s
    // `KNOWN_NONCOMPLIANT` skip-list) rather than mislabeled with a
    // classification tag it doesn't actually carry. Migrating this admission
    // check onto the walk is tracked as deferred work in
    // `docs/plans/20260707-property-composition-walk.md` (see "Deferred
    // during implementation") and `docs/specs/model_properties.md` §Known
    // Divergences ("Heuristic text-scanning layer").
    let upper_sql = sql.to_uppercase();
    if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
        diagnostics.push(KeyedDiagnostic::KeyedForbidsWindowFunctions);
    }

    // Rule: no non-deterministic functions in the outer body.
    for nd in NONDETERMINISTIC_FUNCTIONS {
        let pattern = format!("{}(", nd);
        if upper_sql.contains(&pattern) {
            diagnostics.push(KeyedDiagnostic::KeyedForbidsNondeterministic {
                offending: nd.to_string(),
            });
            break;
        }
    }

    // Rule: GROUP BY must not contain the driving source's partition column
    // — narrowed to models with no `timeseries:` block of their own
    // (`docs/specs/incremental_models.md` §"Key temporal locality"). A
    // keyed model whose GROUP BY includes the partition column AND that
    // declares its own `timeseries:` is not this rule's concern: it is a
    // candidate for the key-embedded locality route (route 1 —
    // `partition_column` is a `unique_key` column), decided by the locality
    // gate (`maintenance::locality::establish_locality`), not refused here.
    if !model_has_timeseries {
        if let Some(ts) = driving_source
            .as_ref()
            .and_then(|ds| ds.timeseries.as_ref())
        {
            let partition_col = &ts.partition_column;
            let partition_col_lower = partition_col.to_ascii_lowercase();
            let contains_partition = unique_key
                .iter()
                .any(|k| k.to_ascii_lowercase() == partition_col_lower)
                || analysis
                    .group_by_exprs
                    .iter()
                    .any(|e| e.to_ascii_lowercase() == partition_col_lower);
            if contains_partition {
                diagnostics.push(KeyedDiagnostic::KeyedGroupByContainsPartitionColumn {
                    partition_column: partition_col.clone(),
                });
            }
        }
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(CumulativeClassification {
        unique_key,
        aggregator_columns,
        driving_source: driving_source.expect("driving_source must be set when diagnostics empty"),
    })
}

/// Classify one `MAX_BY(value, ordering)`/`MIN_BY(value, ordering)`
/// projection (`ArgMax`/`ArgMin`, `Monotone::Order`) — the order-monotone
/// overwrite family (`incremental_models.md` §"The column-family
/// catalogue").
///
/// **Storage decision (Phase 1).** The cross-window combiner needs the
/// *stored* ordering value to compare a new delta's ordering against — but
/// this classifier has no decomposed/hidden-state mechanism (that is
/// rung-2 ladder territory, out of scope here). The only honest option
/// without one is to require the ordering value to already be a genuine
/// output column: this function requires the SELECT list to *also* project
/// `MAX(<ordering>)` (for `MAX_BY`) or `MIN(<ordering>)` (for `MIN_BY`) —
/// the running extremal fold of the exact same ordering expression, which
/// is provably equal to the ordering of MAX_BY/MIN_BY's own winning row by
/// construction. Absent that companion column, the model is refused
/// (`KeyedUnknownCombiner`, naming the missing companion projection) rather
/// than silently deriving a hidden state column.
#[allow(clippy::too_many_arguments)]
fn classify_order_monotone_column(
    analysis: &crate::analysis::SelectAnalysis,
    text: &str,
    alias: &str,
    expr: &smelt_parser::Expr,
    sql_fn: SqlFunction,
    agg_upper: &str,
    aggregator_columns: &mut Vec<AggregatorColumn>,
    diagnostics: &mut Vec<KeyedDiagnostic>,
) {
    if !is_direct_function_call(text, agg_upper) {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: format!("composite expression `{}`", text.trim()),
        });
        return;
    }

    let Some(fc) = expr.as_function_call() else {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: text.trim().to_string(),
        });
        return;
    };
    let args = fc.arguments();
    if args.len() != 2 {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: format!(
                "{agg_upper} requires exactly 2 arguments (value, ordering), got {}",
                args.len()
            ),
        });
        return;
    }
    let value_text = args[0].text().trim().to_string();
    let ordering_text = args[1].text().trim().to_string();
    let tracking_fn = if sql_fn == SqlFunction::ArgMax {
        "MAX"
    } else {
        "MIN"
    };

    match order_monotone_companion(
        &analysis.items,
        alias,
        &value_text,
        &ordering_text,
        tracking_fn,
    ) {
        Some(ordering_column) => {
            aggregator_columns.push(AggregatorColumn {
                output_name: alias.to_string(),
                per_partition_agg: agg_upper.to_string(),
                cross_partition_combiner: CrossPartitionCombiner::OrderMonotone { ordering_column },
            });
        }
        None => {
            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                projection: alias.to_string(),
                offending: format!(
                    "{agg_upper}'s ordering expression `{ordering_text}` must also be \
                     projected as `{tracking_fn}({ordering_text})` — the classifier stores no \
                     hidden ordering state, only what the SELECT already projects"
                ),
            });
        }
    }
}

/// Whether an order-monotone (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`)
/// projection's ordering value is provably trustworthy under the storage
/// decision documented on [`CrossPartitionCombiner::OrderMonotone`] — the
/// cross-window combiner has no decomposed/hidden state, so it can only
/// compare the *stored output* of some column in the same projection
/// against the delta's. Two shapes prove that:
///
/// 1. **Self-companion**: the value expression is textually identical to
///    the ordering expression (`MAX_BY(x, x)` ≡ `MAX(x)`) — the projected
///    value already *is* the running extremal of the ordering expression,
///    by construction, so the column is trivially its own companion.
/// 2. **Explicit companion**: another projection in the same SELECT list
///    is a direct `<tracking_fn>(<ordering_text>)` call — the running
///    extremal fold of the exact same ordering expression.
///
/// Returns the alias of the column whose `target.<alias>`/`delta.<alias>`
/// the cross-partition merge should compare (the projection's own `alias`
/// in the self-companion case, the companion projection's alias
/// otherwise). `None` means neither shape is provable from the SELECT's
/// own projections — the projection must be refused, not admitted with a
/// hidden ordering column.
///
/// **Single ownership** (`CLAUDE.md` §"Fail-loud discipline"): both the
/// runtime classifier ([`classify_order_monotone_column`], which
/// admits/refuses at execution time with `KeyedUnknownCombiner`) and the
/// plan-derivation layer (`smelt_db::queries::maintenance::derive_fold_spec`,
/// which decides whether an `ArgMax`/`ArgMin` column enters a `FoldSpec` at
/// all) call this one helper, so `smelt explain`/LSP diagnostics never
/// report a fold admission the runtime then refuses.
pub fn order_monotone_companion(
    items: &[SelectItemKind],
    alias: &str,
    value_text: &str,
    ordering_text: &str,
    tracking_fn: &str,
) -> Option<String> {
    if value_text.trim() == ordering_text.trim() {
        return Some(alias.to_string());
    }
    for item in items {
        let SelectItemKind::OtherAggregate {
            text, alias, expr, ..
        } = item
        else {
            continue;
        };
        if !is_direct_function_call(text, tracking_fn) {
            continue;
        }
        let Some(fc) = expr.as_function_call() else {
            continue;
        };
        let args = fc.arguments();
        if args.len() != 1 {
            continue;
        }
        if args[0].text().trim() == ordering_text {
            return Some(alias.clone());
        }
    }
    None
}

/// The `(family name, refusal reason)` the admission matrix
/// (`docs/specs/incremental_models.md` §"Admission matrix") names for a
/// fold-family aggregator refused under the snapshot-reconcile run shape.
/// `agg_upper` is one of `combiner_for`'s allowlisted names — the additive
/// family (`SUM`/`COUNT`/`BIT_XOR`) double-counts on a re-fold; the
/// remaining extremal/lattice family computes a history observation instead
/// of the current value.
fn snapshot_refusal_reason(agg_upper: &str) -> (&'static str, &'static str) {
    match agg_upper {
        "SUM" | "COUNT" | "BIT_XOR" => (
            "additive fold",
            "re-folding state double-counts — a mutable snapshot is not a replayable, \
             retraction-free event feed",
        ),
        _ => (
            "extremal/lattice fold",
            "observer semantics — folding successive snapshots computes the extremal value \
             ever observed, not the current one (e.g. `MIN(price)` folded over snapshots is \
             the min ever seen, not the current min)",
        ),
    }
}

/// Verify the projection is a direct call to `expected_fn` and nothing else —
/// i.e. it starts with `<fn>(` (after trimming) and the closing `)` is the
/// last non-whitespace character.
fn is_direct_function_call(text: &str, expected_fn: &str) -> bool {
    let trimmed = text.trim();
    let upper = trimmed.to_ascii_uppercase();
    let prefix = format!("{}(", expected_fn);
    // Allow qualified prefixes like `PG_CATALOG.SUM(`.
    let starts_ok = upper.starts_with(&prefix)
        || upper
            .rsplit('.')
            .next()
            .map(|s| s.starts_with(&prefix))
            .unwrap_or(false);
    if !starts_ok {
        return false;
    }
    // The expression must terminate at the matching close-paren.
    if !trimmed.ends_with(')') {
        return false;
    }
    // Confirm the close-paren matches the open-paren of the outer call —
    // i.e. the parenthesis depth returns to zero exactly at the end.
    let mut depth: i32 = 0;
    let mut closed_at_end = false;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    closed_at_end = i + ch.len_utf8() == trimmed.len();
                    break;
                }
            }
            _ => {}
        }
    }
    closed_at_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::Granularity;

    fn ts(partition_col: &str) -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: partition_col.to_string(),
            partition_column: partition_col.to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }
    }

    fn events_source_map() -> SourceTimeseriesMap {
        let mut m = HashMap::new();
        m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
        m
    }

    /// The motivating device_user_edges SELECT classifies cleanly.
    #[test]
    fn test_classify_simple() {
        let sql = r#"SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id"#;

        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification =
            classify_cumulative(sql, &refs, &events_source_map(), false).expect("must classify");

        assert_eq!(classification.unique_key, vec!["device_id", "user_id"]);
        assert_eq!(classification.aggregator_columns.len(), 3);
        let event_count = &classification.aggregator_columns[0];
        assert_eq!(event_count.output_name, "event_count");
        assert_eq!(event_count.per_partition_agg, "COUNT");
        assert_eq!(
            event_count.cross_partition_combiner,
            CrossPartitionCombiner::Sum
        );
        let first_seen = &classification.aggregator_columns[1];
        assert_eq!(first_seen.output_name, "first_seen");
        assert_eq!(first_seen.per_partition_agg, "MIN");
        assert_eq!(
            first_seen.cross_partition_combiner,
            CrossPartitionCombiner::Min
        );
        let last_seen = &classification.aggregator_columns[2];
        assert_eq!(last_seen.output_name, "last_seen");
        assert_eq!(last_seen.per_partition_agg, "MAX");
        assert_eq!(
            last_seen.cross_partition_combiner,
            CrossPartitionCombiner::Max
        );
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// A SELECT with no GROUP BY produces KeyedRequiresGroupBy.
    #[test]
    fn test_no_group_by_refused() {
        let sql = "SELECT COUNT(*) AS n FROM smelt.silver.events_parsed";
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedRequiresGroupBy)),
            "diagnostics: {:?}",
            err
        );
    }

    /// STRING_AGG on a non-key projection produces KeyedUnknownCombiner.
    #[test]
    fn test_unknown_aggregator_refused() {
        let sql = r#"SELECT
    device_id,
    STRING_AGG(name, ',') AS names
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                    if offending.to_uppercase() == "STRING_AGG"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// A composite aggregate expression `SUM(x) + 1` is refused.
    #[test]
    fn test_composite_aggregate_expression_refused() {
        let sql = r#"SELECT
    device_id,
    SUM(amount) + 1 AS shifted_sum
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                    if offending.contains("composite") || offending.contains("expression")
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// COUNT(DISTINCT x) is refused — not commutative under merge.
    #[test]
    fn test_count_distinct_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(DISTINCT user_id) AS unique_users
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedUnknownCombiner { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// GROUP BY containing the driving source's partition_column is refused.
    #[test]
    fn test_group_by_contains_partition_column_refused() {
        let sql = r#"SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id, event_date"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { partition_column }
                    if partition_column == "event_date"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// The same GROUP BY-contains-partition-column shape, but the model
    /// declares its own `timeseries:` block: `KeyedGroupByContainsPartitionColumn`
    /// must NOT fire. The model is instead a candidate for the key-embedded
    /// locality route (`partition_column` is a `unique_key` column) and is
    /// decided by the locality gate
    /// (`maintenance::locality::establish_locality`), not refused here
    /// (`docs/specs/incremental_models.md` §"Key temporal locality").
    #[test]
    fn test_group_by_contains_partition_column_not_refused_when_model_has_timeseries() {
        let sql = r#"SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id, event_date"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let result = classify_cumulative(sql, &refs, &events_source_map(), true);
        match result {
            // With the check narrowed off, this shape has no other
            // rejection — GROUP BY over three plain columns plus a bare
            // COUNT(*) classifies cleanly, and `event_date` lands in the
            // derived unique_key exactly as it would for any other key
            // column.
            Ok(classification) => {
                assert!(
                    classification.unique_key.iter().any(|k| k == "event_date"),
                    "expected event_date in unique_key: {:?}",
                    classification.unique_key
                );
            }
            Err(diagnostics) => {
                assert!(
                    !diagnostics.iter().any(|d| matches!(
                        d,
                        KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { .. }
                    )),
                    "KeyedGroupByContainsPartitionColumn must not fire when the model \
                     declares its own timeseries: block; diagnostics: {:?}",
                    diagnostics
                );
            }
        }
    }

    /// An OVER (...) projection produces KeyedForbidsWindowFunctions.
    #[test]
    fn test_window_function_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) OVER (PARTITION BY device_id) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedForbidsWindowFunctions)),
            "diagnostics: {:?}",
            err
        );
    }

    /// NOW() in the outer body is refused.
    #[test]
    fn test_nondeterministic_refused() {
        let sql = r#"SELECT
    device_id,
    MAX(event_ts) - NOW() AS stale_for
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedForbidsNondeterministic { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// A SELECT from a single unclocked source now derives the
    /// snapshot-reconcile run shape (Phase 3, `docs/plans/20260809-keyed-
    /// frontier.md`) instead of refusing the whole model outright — but a
    /// `COUNT(*)` additive-fold column is still refused, per column, with
    /// `KeyedSnapshotSourceUnsupportedColumn` naming the double-count
    /// reason.
    #[test]
    fn test_zero_clocked_sources_derives_snapshot_reconcile_but_refuses_fold_column() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.lookup_table".to_string()];
        let err = classify_cumulative(sql, &refs, &HashMap::new(), false).unwrap_err();
        assert!(
            !err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedSnapshotPostureUnsupported)),
            "the posture itself (a single unclocked source) is supportable now: {:?}",
            err
        );
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { family, .. }
                    if family == "additive fold"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// Two timeseries-tagged sources produces KeyedMultipleDrivingSources.
    #[test]
    fn test_multiple_driving_sources_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
JOIN smelt.silver.events_b USING (device_id)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let err = classify_cumulative(sql, &refs, &map, false).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedMultipleDrivingSources { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// A timeseries-tagged ref that is only reached through a subquery (not
    /// one of the top-level FROM/JOIN inputs) is not a driving-fact
    /// candidate — the alias-scoped resolver only considers the joined
    /// inputs of the outer scope, unlike the former ref-count selection
    /// (which would have flat-counted both refs and refused as ambiguous).
    #[test]
    fn test_driving_source_resolved_via_alias_scope_ignores_non_joined_ref() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
WHERE device_id IN (SELECT device_id FROM smelt.silver.events_b)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let classification = classify_cumulative(sql, &refs, &map, false)
            .expect("must classify: only events_a is joined");
        assert_eq!(classification.driving_source.name, "smelt.silver.events_a");
    }

    /// A SELECT from one timeseries source and one lookup classifies cleanly.
    #[test]
    fn test_lookup_source_admitted() {
        let sql = r#"SELECT
    e.device_id,
    e.user_id,
    COUNT(*) AS event_count
FROM smelt.silver.events_parsed e
JOIN smelt.silver.user_lookup l USING (user_id)
WHERE l.is_active
GROUP BY e.device_id, e.user_id"#;
        let refs = vec![
            "smelt.silver.events_parsed".to_string(),
            "smelt.silver.user_lookup".to_string(),
        ];
        let classification =
            classify_cumulative(sql, &refs, &events_source_map(), false).expect("must classify");
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// Cross-partition combiners render correctly.
    #[test]
    fn test_combiner_rendering() {
        assert_eq!(
            CrossPartitionCombiner::Sum.render("target.x", "delta.x"),
            "target.x + delta.x"
        );
        assert_eq!(
            CrossPartitionCombiner::Min.render("target.x", "delta.x"),
            "LEAST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::Max.render("target.x", "delta.x"),
            "GREATEST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::BoolOr.render("target.x", "delta.x"),
            "target.x OR delta.x"
        );
    }

    /// combiner_for handles case-insensitive lookups.
    #[test]
    fn test_combiner_lookup_case_insensitive() {
        assert_eq!(combiner_for("count"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("COUNT"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("Sum"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("avg"), None);
        assert_eq!(combiner_for("string_agg"), None);
    }
}
