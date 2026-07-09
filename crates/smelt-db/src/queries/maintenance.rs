//! Thin Salsa wrapper around `smelt_logical::maintenance::derive::derive_maintenance_plan`
//! (`maintenance_plan.md` §Surface "The plan (derived, reported)").
//!
//! Per the Salsa purity rule (`architecture.md` §"Salsa purity rule
//! (analysis)"), this module only *assembles inputs* — resolved source
//! facts, the declared output shape, the derived column groups/skeleton —
//! and calls the pure derivation in `smelt-logical`. It never re-implements
//! admission, locality, or ledger logic itself. The `#[salsa::tracked]`
//! query in `smelt-db/src/lib.rs` (`maintenance_plan`) is the only caller;
//! everything below is a plain function so it can be unit-tested without a
//! Salsa database.

use std::collections::HashMap;

use smelt_core::config::{
    Grain as ConfigGrain, MaintenanceConfig, RefreshStrategy, ScanBoundsConfig, ScanBoundsRequire,
};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelMetadata;
use smelt_logical::analysis::{select_stmt_items, SelectItemKind};
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::skeleton::skeleton_columns;
use smelt_logical::maintenance::{
    ColumnGroup, Grain as PlanGrain, MaintenancePlan, MutationProfile as PlanMutationProfile,
    OutputSpec, SourceFacts, Trigger,
};
use smelt_types::SqlFunction;

/// Everything `maintenance_plan` derives for one model: the raw plan (cells
/// and admission refusals) plus the column groups the `maintenance.cells[]`
/// frontmatter check reuses — a single derivation feeds both, per the
/// maintenance-plan-purity invariant ("derived once by pure functions;
/// consumers never re-derive it").
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlanResult {
    pub plan: MaintenancePlan,
    pub column_groups: Vec<ColumnGroup>,
}

/// Build one [`SourceFacts`] from a resolved source declaration (`None` when
/// the ref did not resolve to a known source) and the effective
/// `allow_full_scan` acceptance for it.
pub fn source_facts(name: &str, info: Option<&SourceInfo>, allow_full_scan: bool) -> SourceFacts {
    let partition_col = info
        .and_then(|s| s.timeseries.as_ref())
        .map(|t| t.partition_column.clone());
    let mutation = match info
        .and_then(|s| s.mutation_profile.as_ref())
        .map(|m| m.kind)
    {
        Some(SourceMutationKind::AppendOnly) => PlanMutationProfile::AppendOnly,
        // `ChangeFeed` has no plan-layer representation yet
        // (`maintenance_plan.md` §Known Divergences); undeclared and
        // `Mutable` both fail closed to the stricter posture rather than
        // assume append-only.
        _ => PlanMutationProfile::MutableSnapshot,
    };
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col,
        unique_key: vec![],
        allow_full_scan,
    }
}

/// Resolve the effective `maintenance.scan_bounds` for `source_address`:
/// the model's own block wins over the project baseline in `smelt.yml`
/// (`maintenance_plan.md` §Surface "Frontmatter": "A project-level default
/// in `smelt.yml` sets the baseline; per-model blocks refine it").
///
/// Returns `(allow_full_scan, require)`. `on_violation` severity mapping is
/// not yet consumed here — every refusal maps to an Error diagnostic today
/// (`docs/specs/maintenance_plan.md` §Known Divergences narrows this once a
/// consumer needs the Warning path).
pub fn effective_scan_bounds(
    source_address: &str,
    model: Option<&ScanBoundsConfig>,
    project: Option<&ScanBoundsConfig>,
) -> (bool, ScanBoundsRequire) {
    let require = model
        .and_then(|s| s.require)
        .or_else(|| project.and_then(|s| s.require))
        .unwrap_or(ScanBoundsRequire::PartitionLocal);
    // `require: none` disables the guardrail outright: every source reads as
    // though it were explicitly accepted, since the operator declared no
    // partition-locality expectation to check the derived plan against.
    if require == ScanBoundsRequire::None {
        return (true, require);
    }
    let allow = model
        .map(|s| s.allow_full_scan(source_address))
        .filter(|b| *b)
        .or_else(|| project.map(|s| s.allow_full_scan(source_address)))
        .unwrap_or(false);
    (allow, require)
}

/// Detect a single-aggregate fold candidate for a `grain: key` model's own
/// outermost `SELECT`: exactly one non-key-by column that is a direct call
/// to a recognised aggregate function.
///
/// This is *input assembly*, not admission: a wrong or missing guess here
/// only ever narrows to "no fold candidate" (the derivation then refuses
/// the `NewData` cell rather than fabricate one), never admits a fold the
/// derivation doesn't independently re-check via `combiner_discriminants`
/// and the source-posture obligation.
pub fn derive_fold_spec(sql: &str) -> Option<FoldSpec> {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    let mut aggregates: Vec<(String, SqlFunction)> = Vec::new();
    for item in &items {
        if let SelectItemKind::OtherAggregate { alias, expr, .. } = item {
            let Some(func) = expr.as_function_call() else {
                continue;
            };
            let Some(name) = func.name() else { continue };
            let Some(combiner) = SqlFunction::from_name(&name.to_uppercase()) else {
                continue;
            };
            aggregates.push((alias.clone(), combiner));
        }
    }
    if aggregates.len() != 1 {
        return None;
    }
    let (alias, combiner) = aggregates.into_iter().next()?;
    Some(FoldSpec {
        add_columns: vec![alias],
        combiner,
    })
}

/// Assemble [`ModelInputs`] from already-resolved facts and derive the
/// plan. `sql` is the model body with frontmatter stripped. `sources` is
/// every source the model's `FROM`/`JOIN` clauses reference, already
/// resolved by the caller (mirrors `smelt-db::lib::ref_timeseries_config`'s
/// resolution, reused here for `mutation_profile` /
/// `allow_full_scan` instead of just `timeseries`).
///
/// Returns `None` when the model has no maintenance plan to derive: only
/// `refresh: incremental` models carry one (`maintenance_plan.md` §Surface
/// "The plan (derived, reported)": "Every non-`full` model has a
/// maintenance plan").
pub fn derive_model_maintenance_plan(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &std::collections::HashSet<String>,
) -> Option<MaintenancePlanResult> {
    if metadata.refresh != Some(RefreshStrategy::Incremental) {
        return None;
    }
    let grain = metadata.grain?;
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let plan_grain = match grain {
        ConfigGrain::Partition => PlanGrain::Partition {
            partition_col: partition_col.clone().unwrap_or_default(),
        },
        ConfigGrain::Key | ConfigGrain::KeyPerPartition => PlanGrain::Key { unique_key: vec![] },
    };
    let skeleton = skeleton_columns(sql, &[], partition_col.as_deref());
    let grouping = derive_column_groups(sql, sources, &skeleton);
    let fold = match grain {
        ConfigGrain::Key => derive_fold_spec(sql),
        _ => None,
    };
    let output = OutputSpec {
        table: table.to_string(),
        grain: plan_grain,
        skeleton_columns: skeleton,
    };
    let inputs = ModelInputs {
        sql,
        output,
        sources: sources.to_vec(),
        column_groups: grouping.groups.clone(),
        fold,
        column_add_proof: None,
    };

    let mut triggers = Vec::new();
    for s in sources {
        triggers.push(Trigger::NewData {
            source: s.name.clone(),
        });
        // A mutation trigger is derived only for a source that (a) is
        // unclocked (`partition_col: None` — a pure lookup join no clocked
        // read window ever bounds) and (b) **explicitly** declares
        // `mutation_profile: mutable_snapshot` on its own source YAML — not
        // merely the fail-closed default `source_facts` assigns an
        // undeclared source for admission purposes elsewhere. A source that
        // hasn't declared any posture at all is the common case for a
        // pre-existing model with no maintenance-plan awareness yet;
        // treating "undeclared" as "declared mutable" here would newly
        // refuse builds that never opted into the K8 guardrail's scope.
        // Two further cases are deliberately NOT derived here yet:
        //   - An `AppendOnly` source's rows never change once written — the
        //     aggregate-window sensitivity `grouping::derive_column_groups`
        //     marks it with describes the *creation* cell's still-open
        //     window, not a mutation trigger of its own.
        //   - A **clocked** `MutableSnapshot` source (the driving source
        //     itself, or an enrichment join with its own `timeseries:`)
        //     would need the runtime-injected incremental read window to
        //     link a static bound — invisible to `derive_model_bounds`'s
        //     text-level scan — so deriving an `UpstreamMutation` trigger
        //     for it here would spuriously refuse under the K8 guardrail
        //     whenever the model's raw SQL has no literal date predicate
        //     (the normal, correct shape for a window-forward incremental
        //     model). Scoping this trigger to unclocked, explicitly-mutable
        //     sources only is this phase's deliberately narrow slice of
        //     obligation 4 (`maintenance_plan.md` §"Per-cell admission"); a
        //     clocked enrichment join's own scan-bound derivation is
        //     deferred.
        if s.partition_col.is_none()
            && s.mutation == PlanMutationProfile::MutableSnapshot
            && explicitly_mutable.contains(&s.name)
        {
            triggers.push(Trigger::UpstreamMutation {
                source: s.name.clone(),
            });
        }
    }
    triggers.push(Trigger::Backfill);

    let plan = derive_maintenance_plan(&inputs, &triggers);
    Some(MaintenancePlanResult {
        plan,
        column_groups: grouping.groups,
    })
}

/// A `maintenance.cells[]` entry whose declared `columns` span more than one
/// derived column group — an error, since it would silently re-partition
/// the plan (`maintenance_plan.md` §Surface "Frontmatter"). Returns one
/// message per offending cell, naming the cell's `on:` trigger and the
/// distinct group names its columns land in.
pub fn cell_column_group_violations(
    maintenance: &MaintenanceConfig,
    groups: &[ColumnGroup],
) -> Vec<String> {
    let mut violations = Vec::new();
    for cell in &maintenance.cells {
        let mut hit_groups: Vec<String> = Vec::new();
        for col in &cell.columns {
            if let Some(group) = groups.iter().find(|g| g.columns.iter().any(|c| c == col)) {
                let name = group.name();
                if !hit_groups.contains(&name) {
                    hit_groups.push(name);
                }
            }
        }
        if hit_groups.len() > 1 {
            violations.push(format!(
                "maintenance.cells[on: {}].columns spans {} derived column groups ({}); \
                 a cell must address exactly one group — split it into one cell per group",
                cell.on,
                hit_groups.len(),
                hit_groups.join(", "),
            ));
        }
    }
    violations
}

/// Build [`SourceFacts`] for every `(ref_string, source_info)` pair a model
/// references, applying the `maintenance.scan_bounds` ladder.
pub fn build_source_facts(
    refs: &[(String, Option<SourceInfo>)],
    model_scan_bounds: Option<&ScanBoundsConfig>,
    project_scan_bounds: Option<&ScanBoundsConfig>,
) -> Vec<SourceFacts> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    for (name, info) in refs {
        if seen.contains_key(name) {
            continue;
        }
        seen.insert(name.clone(), ());
        let (allow_full_scan, _require) =
            effective_scan_bounds(name, model_scan_bounds, project_scan_bounds);
        out.push(source_facts(name, info.as_ref(), allow_full_scan));
    }
    out
}

/// A Salsa-friendly (`PartialEq`) projection of a
/// [`smelt_logical::maintenance::Refusal`] — the two refusal kinds this
/// phase maps onto `Maintenance*` diagnostics. Mirrors the pure `Refusal`
/// enum's data exactly; it exists only so `MaintenancePlanDiagnostics` can
/// be a Salsa tracked-query return value without requiring `PartialEq` on
/// every type in `smelt-logical::maintenance` (out of this phase's allowed
/// files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceRefusal {
    ScanUnbounded { source: String, why: String },
    NoAdmissibleTechnique { trigger: String, why: String },
}

/// The result `maintenance_plan` (the Salsa query) returns: every admission
/// refusal from the derived plan, mapped to a Salsa-safe shape, plus the
/// `maintenance.cells[]` column-group-span violations. `file_diagnostics`
/// folds both into `Maintenance*` diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintenancePlanDiagnostics {
    pub refusals: Vec<MaintenanceRefusal>,
    pub cell_column_group_violations: Vec<String>,
}

/// Assemble inputs (resolved source facts, declared output shape,
/// `maintenance.cells[]`) and derive the plan, mapping its refusals into
/// [`MaintenancePlanDiagnostics`]. `source_refs` is every `smelt.<path>`
/// this model's SQL references that resolves to a source declaration
/// (already resolved by the caller — mirrors
/// `smelt-db::lib::ref_timeseries_config`'s resolution seam).
///
/// Pure function — the `#[salsa::tracked]` wrapper in `smelt-db/src/lib.rs`
/// only gathers `source_refs`/`metadata`/`sql` and calls this.
pub fn maintenance_plan_diagnostics(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    source_refs: &[(String, Option<SourceInfo>)],
    project_scan_bounds: Option<&ScanBoundsConfig>,
) -> MaintenancePlanDiagnostics {
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let sources = build_source_facts(source_refs, model_scan_bounds, project_scan_bounds);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == SourceMutationKind::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    let Some(result) =
        derive_model_maintenance_plan(sql, table, metadata, &sources, &explicitly_mutable)
    else {
        return MaintenancePlanDiagnostics::default();
    };
    let refusals = result
        .plan
        .refusals
        .iter()
        .filter_map(|r| match r {
            smelt_logical::maintenance::Refusal::ScanUnbounded { source, why } => {
                Some(MaintenanceRefusal::ScanUnbounded {
                    source: source.clone(),
                    why: why.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, why } => {
                Some(MaintenanceRefusal::NoAdmissibleTechnique {
                    trigger: trigger.clone(),
                    why: why.clone(),
                })
            }
            // A definition-change grain refusal — not reachable in this
            // phase (no `ColumnAdded` trigger is derived yet); leave
            // unmapped so a future phase's own diagnostic lands it.
            smelt_logical::maintenance::Refusal::SkeletonColumnAdded { .. } => None,
        })
        .collect();
    let cell_column_group_violations = metadata
        .maintenance
        .as_ref()
        .map(|m| cell_column_group_violations(m, &result.column_groups))
        .unwrap_or_default();
    MaintenancePlanDiagnostics {
        refusals,
        cell_column_group_violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::{MaintenanceCellConfig, PerSourceScanBounds};

    fn group(columns: &[&str], sensitivity: &[&str]) -> ColumnGroup {
        ColumnGroup {
            columns: columns.iter().map(|s| s.to_string()).collect(),
            mutation_sensitivity: sensitivity.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn cells_columns_spanning_groups_error() {
        let groups = vec![
            group(&["converted"], &["payments"]),
            group(&["shipped"], &["shipments"]),
        ];
        let maintenance = MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec!["converted".to_string(), "shipped".to_string()],
                on: "sources.payments".to_string(),
                prefer: None,
                technique: None,
            }],
            scan_bounds: None,
        };
        let violations = cell_column_group_violations(&maintenance, &groups);
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert!(violations[0].contains("sources.payments"));
    }

    #[test]
    fn cells_columns_within_one_group_ok() {
        let groups = vec![group(&["converted", "converted_at"], &["payments"])];
        let maintenance = MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec!["converted".to_string(), "converted_at".to_string()],
                on: "sources.payments".to_string(),
                prefer: None,
                technique: None,
            }],
            scan_bounds: None,
        };
        assert!(cell_column_group_violations(&maintenance, &groups).is_empty());
    }

    #[test]
    fn allow_full_scan_true_clears_scan_unbounded() {
        let sources = [source_facts("sources.enrichment", None, false)];
        assert!(!sources[0].allow_full_scan);
        let sources = [source_facts("sources.enrichment", None, true)];
        assert!(sources[0].allow_full_scan);
    }

    #[test]
    fn effective_scan_bounds_model_overrides_project() {
        let mut project = ScanBoundsConfig::default();
        project.per_source.insert(
            "sources.enrichment".to_string(),
            PerSourceScanBounds {
                max_lookback: None,
                allow_full_scan: false,
            },
        );
        let mut model = ScanBoundsConfig::default();
        model.per_source.insert(
            "sources.enrichment".to_string(),
            PerSourceScanBounds {
                max_lookback: None,
                allow_full_scan: true,
            },
        );
        let (allow, require) =
            effective_scan_bounds("sources.enrichment", Some(&model), Some(&project));
        assert!(allow);
        assert_eq!(require, ScanBoundsRequire::PartitionLocal);
    }

    #[test]
    fn grain_mismatch_never_admits_fold_without_aggregate() {
        // `grain: key` with a body that never aggregates has no fold
        // candidate — `derive_fold_spec` must return `None`, not fabricate
        // one, so the derivation's own admission refuses honestly.
        let sql = "SELECT user_id, amount FROM smelt.sources.payments";
        assert!(derive_fold_spec(sql).is_none());
    }

    #[test]
    fn grain_mismatch_detects_single_aggregate() {
        let sql =
            "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
        let fold = derive_fold_spec(sql).expect("single SUM aggregate should be a fold candidate");
        assert_eq!(fold.add_columns, vec!["total".to_string()]);
        assert_eq!(fold.combiner, SqlFunction::Sum);
    }
}
