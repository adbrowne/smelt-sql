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

use crate::graph::ModelInfo;
use crate::rules::cumulative::{classify_cumulative, CumulativeDiagnostic, SourceTimeseriesMap};
use crate::rules::incremental;
use crate::types::IncrementalConfig;

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
    CumulativeRequiresGroupBy,
    CumulativeUnknownAggregator,
    CumulativeGroupByContainsPartitionColumn,
    CumulativeForbidsWindowFunctions,
    CumulativeForbidsNondeterministic,
    CumulativeNoDrivingSource,
    CumulativeMultipleDrivingSources,
    CumulativeSqlNotParseable,
    IncrementalNotBatchSafe,
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
    /// `"cumulative_aggregate"` or `"incremental"`. A rule keys its scope off
    /// this exactly as the build does.
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
    pub incremental_config: Option<&'a IncrementalConfig>,
}

/// A planner rule that surfaces its rejections as diagnostics.
///
/// `detect` is sync and side-effect-free. A rule returns an empty vec when the
/// model is outside its scope or clean.
pub trait PlannerRule {
    fn detect(&self, ctx: &RuleContext) -> Vec<RuleDiagnostic>;
}

/// The cumulative-aggregate classifier as a uniform rule.
///
/// Its rejections refuse the model at planning time (`cumulative_aggregate.md`
/// §"Classifier checks"), so every one is `Error` — the build hard-refuses on
/// them today via `smelt_planner::classify_cumulative`.
pub struct CumulativeRule;

impl PlannerRule for CumulativeRule {
    fn detect(&self, ctx: &RuleContext) -> Vec<RuleDiagnostic> {
        if ctx.materialization != "cumulative_aggregate" {
            return Vec::new();
        }
        match classify_cumulative(ctx.sql, ctx.refs, ctx.source_timeseries) {
            Ok(_) => Vec::new(),
            Err(diags) => diags.iter().map(cumulative_to_rule).collect(),
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
/// the frontmatter validator (`TimeseriesRequiredForIncremental`), so this rule
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
        let model = ModelInfo {
            name: ctx.model_name.to_string(),
            sql: ctx.sql.to_string(),
            refs: ctx.refs.to_vec(),
            timeseries_config: Some(ts.clone()),
            incremental_config: Some(inc.clone()),
        };
        match incremental::detect(&model) {
            Ok(_) => Vec::new(),
            Err(message) => vec![RuleDiagnostic {
                code: RuleDiagnosticCode::IncrementalNotBatchSafe,
                severity: RuleSeverity::Warning,
                message,
            }],
        }
    }
}

/// Run every built-in rule applicable to `ctx` and collect their diagnostics.
///
/// This is the single entry point `smelt_db::file_diagnostics` calls; adding a
/// rule here surfaces it to the editor and the build at once.
pub fn detect_builtin_rules(ctx: &RuleContext) -> Vec<RuleDiagnostic> {
    let mut out = Vec::new();
    out.extend(CumulativeRule.detect(ctx));
    out.extend(IncrementalRule.detect(ctx));
    out
}

/// Map a cumulative-classifier diagnostic to its uniform rule diagnostic. Every
/// classifier rejection is `Error`.
fn cumulative_to_rule(diag: &CumulativeDiagnostic) -> RuleDiagnostic {
    let code = match diag {
        CumulativeDiagnostic::CumulativeRequiresGroupBy => {
            RuleDiagnosticCode::CumulativeRequiresGroupBy
        }
        CumulativeDiagnostic::CumulativeUnknownAggregator { .. } => {
            RuleDiagnosticCode::CumulativeUnknownAggregator
        }
        CumulativeDiagnostic::CumulativeGroupByContainsPartitionColumn { .. } => {
            RuleDiagnosticCode::CumulativeGroupByContainsPartitionColumn
        }
        CumulativeDiagnostic::CumulativeForbidsWindowFunctions => {
            RuleDiagnosticCode::CumulativeForbidsWindowFunctions
        }
        CumulativeDiagnostic::CumulativeForbidsNondeterministic { .. } => {
            RuleDiagnosticCode::CumulativeForbidsNondeterministic
        }
        CumulativeDiagnostic::CumulativeNoDrivingSource => {
            RuleDiagnosticCode::CumulativeNoDrivingSource
        }
        CumulativeDiagnostic::CumulativeMultipleDrivingSources { .. } => {
            RuleDiagnosticCode::CumulativeMultipleDrivingSources
        }
        CumulativeDiagnostic::SqlNotParseable => RuleDiagnosticCode::CumulativeSqlNotParseable,
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
/// single source of ref collection shared by the runtime cumulative dispatch
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
    fn cumulative_rule_flags_unknown_aggregator() {
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
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags.iter().any(
                |d| d.code == RuleDiagnosticCode::CumulativeUnknownAggregator
                    && d.severity == RuleSeverity::Error
            ),
            "expected CumulativeUnknownAggregator Error, got {diags:?}"
        );
    }

    #[test]
    fn cumulative_rule_clean_model_is_silent() {
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
        };
        assert!(
            detect_builtin_rules(&ctx).is_empty(),
            "valid cumulative model must produce no rule diagnostics"
        );
    }

    #[test]
    fn non_cumulative_materialization_is_silent() {
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
        };
        assert!(detect_builtin_rules(&ctx).is_empty());
    }

    fn inc_config() -> IncrementalConfig {
        IncrementalConfig {
            enabled: true,
            unique_key: vec!["event_date".to_string()],
            safety_overrides: Default::default(),
        }
    }

    #[test]
    fn incremental_rule_flags_not_batch_safe_as_warning() {
        // Structurally valid incremental model (partition column `event_date`
        // is a SELECT alias and a GROUP BY key), but a HAVING clause makes it
        // not batch-safe → the incremental safety classifier rejects it.
        let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.src \
                   GROUP BY event_date HAVING COUNT(*) > 1";
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
        };
        let diags = detect_builtin_rules(&ctx);
        assert!(
            diags
                .iter()
                .any(|d| d.code == RuleDiagnosticCode::IncrementalNotBatchSafe
                    && d.severity == RuleSeverity::Warning),
            "expected IncrementalNotBatchSafe Warning, got {diags:?}"
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
        };
        assert!(detect_builtin_rules(&ctx).is_empty());
    }
}
