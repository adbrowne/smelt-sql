//! Characterization probes for `docs/outcomes/20260815-partition-grain-residue`,
//! phase 1 (audit). Each probe pins TODAY's behaviour for one partition-grain
//! Known Divergences bullet in `docs/specs/incremental_shapes.md`. A probe
//! that already asserts spec-required behaviour is evidence the residue is
//! closed; see `docs/outcomes/20260815-partition-grain-residue/audit.md` for
//! the full verdict table.

use std::collections::{BTreeSet, HashMap};

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::{
    analyze_batch_safety, collect_path_refs, derive_model_source_bounds, detect_builtin_rules,
    BatchSafety, ModelGraph, ModelInfo, PartitionGrainConfig, RuleContext, RuleDiagnosticCode,
};

fn ts_config(event_time_column: &str, partition_column: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_time_column.to_string(),
        partition_column: partition_column.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn upstream_ts_model(name: &str, event_time_column: &str, partition_column: &str) -> ModelInfo {
    ModelInfo {
        name: name.to_string(),
        sql: String::new(),
        refs: vec![],
        timeseries_config: Some(ts_config(event_time_column, partition_column)),
        incremental_config: None,
        plausible_columns: Default::default(),
    }
}

fn downstream_model(name: &str, sql: &str, refs: Vec<String>) -> ModelInfo {
    ModelInfo {
        name: name.to_string(),
        sql: sql.to_string(),
        refs,
        timeseries_config: Some(ts_config("event_date", "event_date")),
        incremental_config: Some(PartitionGrainConfig {
            unique_key: vec!["event_date".to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: Default::default(),
        }),
        plausible_columns: Default::default(),
    }
}

/// Residue: "One classification call site reads the outer SQL body" —
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known Divergences.
/// Tracked: `docs/plans/20260530-thread-fn-registry-classification.md`. The
/// plan's own "Remaining outer-SQL call sites" section names
/// `derive_model_source_bounds` (called from `commands/run.rs` and the
/// backfill path, still on raw `model.content`) as unfixed. Inverts in phase 2.
///
/// A lookback filter that exists only inside a `smelt.define` function body —
/// simulated here by comparing the bound derived from the *unexpanded* call
/// form against the bound derived from the same query with the body already
/// inlined (what execution actually runs) — must, once fixed, derive the same
/// bound either way. Today they diverge: the gate is blind to the inlined
/// filter and under-derives the lookback on the unexpanded form.
#[test]
fn probe_lookback_gate_sees_define_body() {
    let mut graph = ModelGraph::new();
    graph.add_model(upstream_ts_model(
        "smelt.orders",
        "event_date",
        "event_date",
    ));

    let unexpanded = downstream_model(
        "recent_orders",
        "SELECT * FROM smelt.functions.recent_window(source => smelt.orders)",
        vec!["smelt.orders".to_string()],
    );
    let expanded = downstream_model(
        "recent_orders",
        "SELECT * FROM (SELECT * FROM smelt.orders \
         WHERE event_date >= CURRENT_DATE - INTERVAL '7 day') t",
        vec!["smelt.orders".to_string()],
    );

    let unexpanded_bound = derive_model_source_bounds(&unexpanded, &graph)
        .ok()
        .and_then(|m| m.get("smelt.orders").cloned());
    let expanded_bound = derive_model_source_bounds(&expanded, &graph)
        .ok()
        .and_then(|m| m.get("smelt.orders").cloned());

    // TODAY: the gate cannot see the lookback hidden inside the (unexpanded)
    // function-call form, so it derives a different — narrower — bound than
    // the expanded truth. Once phase 2 threads function-registry expansion
    // through this call site, the two must agree; invert this assertion then.
    assert_ne!(
        unexpanded_bound, expanded_bound,
        "lookback gate already reads through smelt.define bodies (bounds agree) — \
         this residue is LANDED; invert this probe and update \
         docs/specs/incremental_shapes.md's Known Divergences entry"
    );
}

/// Residue: "The window-function batch-safety check runs on unexpanded outer
/// SQL" — `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. Tracked: `docs/plans/20260530-thread-fn-registry-classification.md`.
/// `analyze_batch_safety` calls `analyze_temporal_dependencies`, whose
/// AST-based window scan (`analyze_one_select`) never descends into
/// subqueries, and whose textual `RANGE BETWEEN INTERVAL` fallback scan only
/// helps when a fixed caller pre-expands `smelt.define` bodies into
/// `model.sql` first — `analyze_batch_safety` itself takes only a `ModelInfo`
/// and never expands anything. Inverts in phase 2.
#[test]
fn probe_batch_safety_sees_over_in_define_body() {
    // As authored: the lookback frame lives only inside a `smelt.define`
    // function body — the outer SQL calls the function with no outer Form B
    // filter, exactly the shape `20260530-thread-fn-registry-classification.md`
    // names as "no such case exists in the repo" for the (fixed) explain path.
    let unexpanded = downstream_model(
        "rolling_orders",
        "SELECT * FROM smelt.functions.rolling_7d(source => smelt.orders)",
        vec!["smelt.orders".to_string()],
    );
    // Expanded: the function body inlined, as execution actually runs it.
    let expanded = downstream_model(
        "rolling_orders",
        "SELECT event_date, SUM(amount) OVER (\
         ORDER BY event_date RANGE BETWEEN INTERVAL '7 day' PRECEDING AND CURRENT ROW) AS total \
         FROM smelt.orders",
        vec!["smelt.orders".to_string()],
    );

    let unexpanded_safety = analyze_batch_safety(&unexpanded);
    let expanded_safety = analyze_batch_safety(&expanded);

    let is_fully_safe = matches!(unexpanded_safety, BatchSafety::FullyBatchSafe);
    let expanded_is_bounded = matches!(expanded_safety, BatchSafety::BoundedSafe { .. });

    // TODAY: `analyze_batch_safety` never expands function calls itself, so
    // the unexpanded form's embedded OVER/RANGE lookback is invisible to it —
    // it classifies `FullyBatchSafe` (no lookback seen) even though the
    // expanded truth is `BoundedSafe` with a real 7-day lookback.
    assert!(
        is_fully_safe && expanded_is_bounded,
        "analyze_batch_safety already sees the define-body OVER/RANGE lookback \
         (unexpanded: {unexpanded_safety:?}, expanded: {expanded_safety:?}) — this \
         residue is LANDED for the bare-analyzer call; invert this probe and update \
         docs/specs/incremental_shapes.md's Known Divergences entry"
    );
}

/// Residue: "CTE-only `event_time_column` references are not yet detected" —
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. Tracked: `docs/plans/20260616-smelt-feedback-fixes.md`.
/// `check_event_time_injectable`'s Case 2 only handles a bare parenthesized
/// subquery in the FROM clause (`from_text.starts_with('(')`) — a `WITH ...
/// AS (...)` CTE alias referenced from FROM never starts with `(`, so this
/// case silently returns `None` and the model is accepted. Inverts in phase 3.
#[test]
fn probe_cte_only_event_time_column() {
    let sql = "WITH recent AS (SELECT user_id, amount FROM smelt.orders) \
               SELECT user_id, SUM(amount) AS total FROM recent GROUP BY user_id";
    let refs = collect_path_refs(sql);
    let ts_cfg = ts_config("event_ts", "event_ts");
    let inc_cfg = PartitionGrainConfig {
        unique_key: vec!["user_id".to_string()],
        nondeterministic_columns_retired: (),
        safety_overrides: Default::default(),
    };
    let ts_map: HashMap<String, TimeseriesConfig> = HashMap::new();
    let ctx = RuleContext {
        model_name: "cte_mart",
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

    // `recent` never projects `event_ts`, so injecting a WHERE filter on the
    // outer SELECT (which only sees `user_id`/`total`) cannot work — the spec
    // requires `EventTimeColumnNotVisibleAtOuterSelect` here. TODAY the CTE
    // form escapes the bare-subquery-only check and is accepted.
    let caught = diags
        .iter()
        .any(|d| d.code == RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect);
    assert!(
        !caught,
        "CTE-only event_time_column non-visibility is already caught \
         (EventTimeColumnNotVisibleAtOuterSelect fired) — this residue is LANDED; \
         invert this probe and update docs/specs/incremental_shapes.md's \
         Known Divergences entry"
    );
}

/// Residue: "Per-`ModelDef` overrides for generator-emitted models are not
/// part of the closed field set in v1" — `docs/specs/incremental_shapes.md`
/// §"The partition grain" Known Divergences. Tracked:
/// `docs/plans/20260509-meta-language-overall.md` (folded into Phase E2,
/// which explicitly deferred "Per-`ModelDef` frontmatter beyond the closed
/// five-field set" as out of scope). Inverts in phase 4.
#[test]
fn probe_modeldef_per_model_override() {
    let fields = &*smelt_types::signatures::MODEL_DEF_FIELDS;
    let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();

    // The closed five-field set per Phase E2: name, body, materialization,
    // tags, description. No override/config-carrying field exists.
    assert_eq!(
        names,
        vec!["name", "body", "materialization", "tags", "description"],
        "MODEL_DEF_FIELDS grew a field beyond the closed five-field set — if this \
         is a per-model override field, this residue is LANDED; invert this probe \
         and update docs/specs/incremental_shapes.md's Known Divergences entry"
    );
}
