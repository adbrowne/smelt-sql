use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::Utc;
use tracing::warn;

use crate::compile::build_source_bound_map;
use crate::transformer::{
    inject_source_filters, inject_time_filter, is_transparent_single_source,
    pin_run_deterministic_clocks, TimeRange,
};

/// Per-model source-scan bound map (INTERVAL-derived lookback per upstream
/// timeseries source), the input `derive_batch_filtered_sql` needs to clamp a
/// batch's read + write. Mirrors the real run's own inline derivation so the
/// dry-run statement-emission branch clamps a batch identically to a live run
/// (`docs/specs/cli.md` §"`--dry-run` prints the maintenance statements").
///
/// `pub`: also reused by `smelt-cli`'s `explain --show-sql` statement
/// emission (`crates/smelt-cli/src/commands/explain.rs`), which must derive a
/// cell's per-source scan margin identically to a live run — the single-owner
/// derivation this function already is, never re-implemented at the call site.
pub fn build_model_source_bounds(
    model_file: &smelt_core::ModelFile,
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    model_name: &str,
) -> HashMap<String, crate::transformer::SourceBound> {
    let sql_for_bounds = smelt_parser::strip_frontmatter(&model_file.content);
    let model_ref_paths: HashSet<String> = model_file
        .refs
        .iter()
        .map(|r| format!("smelt.{}", r.smelt_ref.to_path().join(".")))
        .collect();
    let dep_ts: HashMap<String, (Vec<String>, String)> = source_timeseries
        .iter()
        .filter(|(smelt_ref, _)| model_ref_paths.contains(*smelt_ref))
        .filter_map(|(smelt_ref, ts)| {
            let path = smelt_ref.strip_prefix("smelt.")?;
            let segs: Vec<String> = path.split('.').map(String::from).collect();
            Some((smelt_ref.clone(), (segs, ts.partition_column.clone())))
        })
        .collect();
    let horizon_ceiling = model_file
        .metadata
        .as_ref()
        .and_then(|m| m.horizon_ceiling.as_ref());
    let (bounds, warnings) = build_source_bound_map(&sql_for_bounds, &dep_ts, horizon_ceiling);
    for warning in &warnings {
        warn!("model '{model_name}': {warning}");
    }
    bounds
}

/// Derive the source-clamped, output-clamped, clock-pinned SQL a single
/// incremental batch reads/writes — the two-layer widened-scan + exact output
/// clamp of `docs/specs/model_transforms.md` §"Source-filter pushdown + the
/// two clamps". Shared by the real run and the `--dry-run` statement-emission
/// branch so the statements a dry-run reports are derived exactly as a live run
/// derives the ones it executes (`docs/specs/cli.md` §"`--dry-run` prints the
/// maintenance statements").
///
/// `skew` is the model's own derived partition-column skew bound
/// (`IncrementalPlan::skew`, sourced from `windowing::compute_incremental_windows`
/// — never re-derived here, maintenance-plan purity). The transparent-slice
/// fast path (`is_transparent_single_source`) additionally requires
/// `skew == Skew::ZERO`: for a skewed model the per-source pushdown filter
/// and the output clamp are genuinely different ranges (the source filter is
/// built from `run_range`, i.e. this batch's own derived-output-window slice,
/// while a *different* batch's scan may reach into this one's margin) even
/// when there is exactly one zero-margin source, so the outer clamp stays
/// load-bearing (`docs/specs/model_transforms.md` §Semantics "Source-filter
/// pushdown + the two clamps").
///
/// `pub`: `smelt-cli`'s `explain --show-sql` statement emission
/// (`crates/smelt-cli/src/commands/explain.rs`) calls this directly so the
/// statements it reports for a `--period`-derived window are built by the
/// exact same single-owner derivation a live run uses — never a second,
/// hand-rolled clamp/pushdown composition at the CLI call site.
pub fn derive_batch_filtered_sql(
    clean_sql: &str,
    partition_col: &str,
    per_model_source_bounds: &HashMap<String, crate::transformer::SourceBound>,
    run_range: &TimeRange,
    run_start: chrono::DateTime<Utc>,
    skew: smelt_logical::analysis::source_bounds::Skew,
) -> Result<String> {
    let filtered_sql = if is_transparent_single_source(per_model_source_bounds)
        && skew == smelt_logical::analysis::source_bounds::Skew::ZERO
    {
        inject_source_filters(clean_sql, per_model_source_bounds, run_range)
    } else {
        let filtered_sql = inject_time_filter(clean_sql, partition_col, run_range)?;
        inject_source_filters(&filtered_sql, per_model_source_bounds, run_range)
    };
    Ok(pin_run_deterministic_clocks(&filtered_sql, run_start))
}

/// Build the project-wide `smelt.<path> → timeseries` lookup map used by
/// the planner (keyed classification) and the incremental execute path
/// (source-filter pushdown, Phase 3).
///
/// Merges two sources of timeseries declarations:
/// 1. **Model-frontmatter** — an incremental model whose output partitions by
///    a time column is itself a timeseries source for downstream consumers.
/// 2. **Source YAML** — per-entity sources declaring a `timeseries:` block
///    become pushdown candidates for incremental models reading them (BUG-072).
///
/// In valid workspaces a model and a source cannot share the same `smelt.<path>`
/// address (address-uniqueness constraint). If they did, the source YAML entry
/// wins (it is inserted last); that is documented here as a design decision
/// pending a normative spec ruling.
pub fn build_source_timeseries_map(
    graph: &smelt_core::graph::DependencyGraph,
    source_infos: &[smelt_core::SourceInfo],
) -> smelt_planner::SourceTimeseriesMap {
    let mut map = smelt_planner::SourceTimeseriesMap::new();

    // Model-frontmatter entries. `unwrap_or_default`: if the graph is cyclic,
    // `execution_order` would fail, but the caller's planner-safety gate already
    // catches cycles before this function is reached, so the fallback is a
    // degenerate safety net.
    let exec_order = graph.execution_order().unwrap_or_default();
    for model_name in &exec_order {
        let Ok(model) = graph.get_model(model_name) else {
            continue;
        };
        if let Some(ts) = model.metadata.as_deref().and_then(|m| m.timeseries.clone()) {
            map.insert(format!("smelt.{}", model.address_segments.join(".")), ts);
        }
    }

    // Source YAML entries (BUG-072 / Phase 2).
    for source in source_infos {
        if let Some(ts) = &source.timeseries {
            map.insert(
                format!("smelt.{}", source.address_segments.join(".")),
                ts.clone(),
            );
        }
    }

    map
}

/// Classify every `refresh: keyed` model in `models` and collect which of
/// them carry at least one aggregator column with decomposed state
/// (`AggregatorColumn.state.is_some()`) — the set `SqlCompiler::
/// set_state_bearing_models_all` needs so a downstream `SELECT *` never
/// surfaces `__part` state columns (`docs/specs/incremental_models.md`
/// §"Decomposed state (rung 2) in keyed models" → "Presentation
/// projection"). A model that fails classification is simply excluded
/// (its own classifier error surfaces separately, on the path that
/// actually maintains it — this map only feeds *consumers'* wildcard
/// rewrites, so a producer-side rejection here must not derail an
/// unrelated compile).
///
/// Non-empty for the order-monotone overwrite family (`MAX_BY`/`MIN_BY`),
/// the once-write family's fallback/multi-candidate spellings, and the
/// decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`) — every family
/// `docs/outcomes/20260809-rung2-state-shapes` has widened admission onto
/// the decomposed-state mechanism for.
pub(crate) fn build_state_bearing_models(
    models: &[smelt_core::ModelFile],
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for model in models {
        let metadata = model.metadata.as_deref();
        if !metadata.is_some_and(|m| m.is_keyed()) {
            continue;
        }
        let clean_sql = smelt_parser::strip_frontmatter(&model.content);
        let model_has_timeseries = metadata.is_some_and(|m| m.timeseries.is_some());
        let declared_fds: &[smelt_core::config::FunctionalDependency] = metadata
            .map(|m| m.functional_dependencies.as_slice())
            .unwrap_or(&[]);
        let Ok(classification) = crate::cumulative::classify_cumulative_sql(
            &model.name,
            &clean_sql,
            source_timeseries,
            model_has_timeseries,
            declared_fds,
        ) else {
            continue;
        };
        let is_state_bearing = classification
            .aggregator_columns
            .iter()
            .any(|col| col.state.is_some());
        if is_state_bearing {
            out.insert(model.name.clone());
        }
    }
    out
}

/// Build the project-wide `smelt.<path> → key_recurrence` lookup map —
/// the sibling of [`build_source_timeseries_map`] over the same
/// `source_infos`, keyed by the same `smelt.<path>` convention (matching
/// `crate::cumulative::CumulativeClassification::driving_source.name`'s own
/// full-address form, not `SourceFacts::name`'s bare form). Consumed only
/// by key temporal locality's route 3 (recurrence-bounded) as the declared
/// fallback (`docs/specs/incremental_shapes.md` §"Key temporal locality") —
/// `crate::cumulative::execute_cumulative_aggregate` looks up its own
/// driving source's entry here.
pub fn build_source_key_recurrence_map(
    source_infos: &[smelt_core::SourceInfo],
) -> HashMap<String, smelt_core::sources::KeyRecurrence> {
    let mut map = HashMap::new();
    for source in source_infos {
        if let Some(kr) = source
            .mutation_profile
            .as_ref()
            .and_then(|m| m.key_recurrence.clone())
        {
            map.insert(format!("smelt.{}", source.address_segments.join(".")), kr);
        }
    }
    map
}
