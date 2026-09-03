//! Characterization probes for `docs/outcomes/20260815-partition-grain-residue`,
//! phase 1 (audit). Each probe pins TODAY's behaviour for one partition-grain
//! Known Divergences bullet in `docs/specs/incremental_shapes.md`. A probe
//! that already asserts spec-required behaviour is evidence the residue is
//! closed; see `docs/outcomes/20260815-partition-grain-residue/audit.md` for
//! the full verdict table.
//!
//! `probe_lookback_gate_sees_define_body` and
//! `probe_batch_safety_sees_over_in_define_body` moved to
//! `crates/smelt-runtime/tests/classification_expansion.rs` in phase 2 —
//! the fix landed at `smelt_runtime::safety::build_model_graph` (the runtime
//! layer that owns function-body expansion), so the inverted, real-fix
//! assertions belong at that layer, not here as bare-`ModelInfo` probes.

use std::collections::{BTreeSet, HashMap};

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::{
    collect_path_refs, detect_builtin_rules, PartitionGrainConfig, RuleContext, RuleDiagnosticCode,
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
