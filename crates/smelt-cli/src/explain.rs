use crate::discovery::ModelFile;
use anyhow::Result;
use serde::Serialize;
use smelt_core::config::{Config, RefreshStrategy, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::sources::SourceInfo;
use smelt_core::{Granularity, Materialization, ModelOriginKind, PartitionGrainConfig};
use smelt_logical::backbuild::MigrationVerdict;
use smelt_logical::maintenance::choice::{resolve_cell_choice, ChosenTechnique};
use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::{MaintenanceDialect, StatementGroup};
use smelt_logical::maintenance::repair::{discovery_posture, RepairDiscoveryPosture};
use smelt_logical::maintenance::{lookup_write_pattern, PlanCell, Technique};
use smelt_planner::{analyze_batch_safety, BatchSafety, BoundContext, BoundResult, ModelInfo};
use smelt_runtime::{CompilerRegistry, EphemeralResolver, SourceBound, TimeRange};
use std::collections::BTreeMap;

/// Top-level JSON output for `smelt explain --json`.
#[derive(Debug, Serialize)]
pub struct ExplainOutput {
    pub models: BTreeMap<String, ExplainModel>,
    pub execution_order: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical: Option<ExplainPhysical>,
}

/// Per-model metadata in the explain output.
#[derive(Debug, Serialize)]
pub struct ExplainModel {
    pub dependencies: Vec<String>,
    pub materialization: Materialization,
    /// Refresh axis: `"keyed"` when the model uses the keyed merge loop
    /// (`materialization: table` + `refresh: keyed`). Omitted
    /// when the model uses the default full-refresh strategy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<ExplainIncremental>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// For generator-emitted models: provenance identifying the generator file
    /// and the `ModelDef.name` that produced this model. Omitted for hand-authored
    /// models (per `docs/specs/cli.md` §"`smelt explain --json` output schema").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ModelOriginKind>,
}

/// Incremental-specific metadata in the explain output.
#[derive(Debug, Serialize)]
pub struct ExplainIncremental {
    pub granularity: Granularity,
    pub partition_column: String,
    pub event_time_column: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unique_key: Vec<String>,
    pub batch_safety: String,
    /// Per-source bound map derived from the model's SQL.
    /// Maps source name → bound result. Only timeseries sources appear;
    /// lookup sources (no `timeseries:`) are absent.
    /// Omitted when the model has no timeseries upstream refs.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub source_bounds: BTreeMap<String, SourceBoundJson>,
}

/// JSON shape for one source's bound in `smelt explain --json`.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceBoundJson {
    Bounded {
        partition_col: String,
        /// ISO-8601 duration (e.g. "P1D", "PT30M", "PT0S").
        before: String,
        /// ISO-8601 duration.
        after: String,
        /// The run-relative scan window `[run_start − before, run_end +
        /// after)`, rendered in the model's own axis domain — resolved via
        /// the same `smelt_logical::resolve_scan_window` a run's pushdown
        /// filter uses. `None` when no `--period` was supplied.
        #[serde(skip_serializing_if = "Option::is_none")]
        scan_start: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scan_end: Option<String>,
        /// Set instead of `scan_start`/`scan_end` when a concrete `--period`
        /// was supplied but the margin could not be resolved to a fixed
        /// value (e.g. a non-uniform month/year offset) — names the reason
        /// rather than guessing a day count.
        #[serde(skip_serializing_if = "Option::is_none")]
        scan_unresolved: Option<String>,
    },
    Unbounded,
    NotDerivable,
}

/// Physical execution plan section of explain output.
#[derive(Debug, Serialize)]
pub struct ExplainPhysical {
    pub execution_order: Vec<String>,
    pub nodes: BTreeMap<String, ExplainPhysicalNode>,
    pub ephemerals: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transformations: Vec<String>,
}

/// Per-node metadata in the physical explain output.
#[derive(Debug, Serialize)]
pub struct ExplainPhysicalNode {
    pub strategy: String,
    pub materialization: Materialization,
    pub target: String,
    #[serde(skip_serializing_if = "is_self_origin")]
    pub logical_origins: Vec<String>,
}

fn is_self_origin(origins: &[String]) -> bool {
    origins.len() == 1
}

// `RelationContractClock`, `RelationContractView`, `RelationContractProvider`,
// `InboundEdgeContract`, and `build_relation_contract` below are now derived
// once in `smelt-runtime::diagnostics`, the shared model-diagnostics builder
// (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder";
// §Semantics "Thin-consumer boundary"). Re-exported here so existing CLI
// imports (`use crate::explain::RelationContractView`, etc.) continue to work
// unchanged — `smelt-cli` no longer owns a second copy of this derivation.
pub use smelt_runtime::diagnostics::{
    build_relation_contract, Admissibility, InboundEdgeContract, ModelDiagnostics,
    PlanCellDiagnostics, PropertySet, RelationContractClock, RelationContractProvider,
    RelationContractView, TechniquePreview,
};

/// Build the [`smelt_planner::BoundContext`] a model's `--json` source-bounds
/// section (`compute_source_bounds`) and the shared `smelt-runtime::
/// diagnostics` property-set derivation both need: one `add_source` per
/// upstream dependency that declares its own `timeseries:` clock. Shared
/// so both call sites build the bound context from the same rule rather
/// than two independent copies of this loop
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer
/// boundary").
pub fn build_bound_context(
    model_name: &str,
    graph: &DependencyGraph,
    config: &Config,
) -> BoundContext {
    let mut ctx = BoundContext::new();
    for dep_name in graph.get_upstream(model_name) {
        if let Ok(dep_model) = graph.get_model(&dep_name) {
            let dep_meta = dep_model.metadata.as_deref();
            let ts = config
                .get_timeseries_with_metadata(&dep_name, dep_meta)
                .cloned()
                .or_else(|| dep_meta.and_then(|m| m.timeseries.clone()));
            if let Some(ts) = ts {
                ctx.add_source(&dep_name, &ts.partition_column);
            }
        }
    }
    ctx
}

/// Render one [`RelationContractView`] as indented text lines shared by
/// the model's own contract and every inbound edge's contract — the same
/// field names (`clock:`, `identity:`, `derived grain:`) print for both
/// providers (`docs/specs/models.md` §"The Relation Contract").
fn write_relation_contract(out: &mut String, indent: &str, contract: &RelationContractView) {
    use std::fmt::Write as _;
    match &contract.clock {
        Some(clock) => {
            let _ = writeln!(
                out,
                "{indent}clock:    event_time_column={} partition_column={} granularity={:?}",
                clock.event_time_column, clock.partition_column, clock.granularity
            );
        }
        None => {
            let _ = writeln!(out, "{indent}clock:    (none)");
        }
    }
    match &contract.identity {
        Some(key) => {
            let _ = writeln!(out, "{indent}identity: {}", key.join(", "));
        }
        None => {
            let _ = writeln!(out, "{indent}identity: (none)");
        }
    }
    match &contract.derived_grain {
        Some(grain) => {
            let _ = writeln!(out, "{indent}derived grain: {grain}");
        }
        None => {
            let _ = writeln!(out, "{indent}derived grain: (unclassified)");
        }
    }
}

/// Render one [`OutputDelta`] verdict as the `delta type:` line's value
/// (`docs/specs/incremental_models.md` §Surface "CLI"): the three lattice
/// names spelled out verbatim, with a `general` verdict additionally naming
/// the construct or world-fact that degraded it.
fn format_output_delta(delta: &smelt_logical::analysis::output_delta::OutputDelta) -> String {
    use smelt_logical::analysis::output_delta::OutputDelta;
    match delta {
        OutputDelta::AppendOnlyWindow { .. } => "append-only within window".to_string(),
        OutputDelta::KeyedUpsert { .. } => "keyed upsert".to_string(),
        OutputDelta::General { reason } => format!("general (degraded by: {reason})"),
    }
}

/// The model's own **delta signature** (`docs/specs/incremental_models.md`
/// §Surface "CLI", Headline bullet): the first thing `smelt explain` prints,
/// in `--json`'s `delta_signature` object and text's `emits:` line alike —
/// one struct renders both surfaces so they cannot drift
/// (`docs/outcomes/20260904-delta-signature-front-door/outcome.md` phase 1).
#[derive(Debug, Clone, Serialize)]
pub struct DeltaSignatureHeadline {
    /// `"append_only_window"` | `"keyed_upsert"` | `"general"`.
    pub shape: String,
    /// `"window"` | `"key"` | `"none"` — `"none"` only for `general`, which
    /// makes no addressing claim.
    pub addressing: String,
    /// Present for `keyed_upsert`: the upsert key columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    /// Present for `append_only_window`, and for `keyed_upsert` once key
    /// temporal locality is admitted: the output axis addressing ranges
    /// over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// Present for `general`: the construct or world-fact that degraded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_by: Option<String>,
    /// Present when key temporal locality is admitted: the rendered slice
    /// bound clause (`"slice-bounded by <axis> under key temporal
    /// locality"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slice_bound: Option<String>,
    /// Present when key temporal locality is admitted: the derived settle
    /// bound (`smelt_logical::maintenance::locality::SettleBound`'s own
    /// `Debug` — route 2's `Never` is printed honestly, never a large
    /// sentinel duration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_bound: Option<String>,
    /// The derived grain friendly label — the SAME string the report's
    /// `derived grain:` row prints (`write_relation_contract`), never a
    /// second label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
}

/// The output axis a [`smelt_logical::maintenance::locality::LocalitySlice`]
/// ranges its slice predicate over — every route carries the same
/// `partition_column` field, just under a different variant.
fn locality_axis(slice: &smelt_logical::maintenance::locality::LocalitySlice) -> &str {
    use smelt_logical::maintenance::locality::LocalitySlice;
    match slice {
        LocalitySlice::Window {
            partition_column, ..
        }
        | LocalitySlice::DeltaValues { partition_column }
        | LocalitySlice::RecurrenceBounded {
            partition_column, ..
        } => partition_column,
    }
}

/// Derive the model's own [`DeltaSignatureHeadline`] (`incremental_models.md`
/// §Surface "CLI"): `shape` is this model's own derived
/// [`smelt_logical::analysis::output_delta::OutputDelta`]
/// (`smelt_db::model_output_delta_for`) — `None` (no derivable output-delta
/// shape at all) renders as a `general` verdict rather than fabricating one.
/// `key_locality` is `result.plan.key_locality` — present only for an
/// admitted `grain: key` + `timeseries:` model, appending the slice-bound
/// and settle-bound clauses the composed-shape worked example shows.
/// `own_contract` supplies the SAME `derived_grain` the report's own
/// Relation Contract block prints.
pub fn delta_signature_headline(
    shape: Option<&smelt_logical::analysis::output_delta::OutputDelta>,
    key_locality: Option<&smelt_logical::maintenance::KeyLocality>,
    own_contract: &RelationContractView,
) -> DeltaSignatureHeadline {
    use smelt_logical::analysis::output_delta::OutputDelta;

    let grain = own_contract.derived_grain.map(|g| g.to_string());
    let (slice_bound, settle_bound, locality_axis_str) = match key_locality {
        Some(locality) => {
            let axis = locality_axis(&locality.slice).to_string();
            (
                Some(format!(
                    "slice-bounded by {axis} under key temporal locality"
                )),
                Some(format!("{:?}", locality.settle_bound)),
                Some(axis),
            )
        }
        None => (None, None, None),
    };

    match shape {
        Some(OutputDelta::KeyedUpsert { keys }) => DeltaSignatureHeadline {
            shape: "keyed_upsert".to_string(),
            addressing: "key".to_string(),
            keys: Some(keys.clone()),
            axis: locality_axis_str,
            degraded_by: None,
            slice_bound,
            settle_bound,
            grain,
        },
        Some(OutputDelta::AppendOnlyWindow { axis }) => DeltaSignatureHeadline {
            shape: "append_only_window".to_string(),
            addressing: "window".to_string(),
            keys: None,
            axis: Some(axis.clone()),
            degraded_by: None,
            slice_bound: None,
            settle_bound: None,
            grain,
        },
        Some(OutputDelta::General { reason }) => DeltaSignatureHeadline {
            shape: "general".to_string(),
            addressing: "none".to_string(),
            keys: None,
            axis: None,
            degraded_by: Some(reason.clone()),
            slice_bound: None,
            settle_bound: None,
            grain,
        },
        None => DeltaSignatureHeadline {
            shape: "general".to_string(),
            addressing: "none".to_string(),
            keys: None,
            axis: None,
            degraded_by: Some("no derivable output-delta shape".to_string()),
            slice_bound: None,
            settle_bound: None,
            grain,
        },
    }
}

impl DeltaSignatureHeadline {
    /// Render as the report's headline `(emits: …; grain: …)` clause — the
    /// same text both the plain-text report and (indirectly, since
    /// `--json`'s consumer can reconstruct it from the same fields)
    /// `--json` describe.
    pub fn render_text(&self) -> String {
        use std::fmt::Write as _;
        let mut emits = match self.shape.as_str() {
            "keyed_upsert" => {
                let keys = self.keys.as_deref().unwrap_or_default().join(", ");
                format!("keyed upsert over [{keys}], key-addressed")
            }
            "append_only_window" => {
                let axis = self.axis.as_deref().unwrap_or("?");
                format!("append-only within a window, window-addressed by {axis}")
            }
            _ => {
                let reason = self.degraded_by.as_deref().unwrap_or("unclassified");
                format!("general (degraded by: {reason}), not delta-addressable")
            }
        };
        if let (Some(slice_bound), Some(settle_bound)) = (&self.slice_bound, &self.settle_bound) {
            let _ = write!(emits, ", {slice_bound} (settle bound: {settle_bound})");
        }
        match &self.grain {
            Some(grain) => format!("(emits: {emits}; grain: {grain})"),
            None => format!("(emits: {emits})"),
        }
    }
}

/// A cell's own trigger, addressed the same way `maintenance.cells[]`/
/// `contract.cells[]` address it (a source address, or the literal
/// `backfill`) — `None` for `Trigger::ColumnAdded`, which has no `on:`
/// address of its own. Single-owned in `smelt_logical::maintenance`
/// (`cell_trigger_address`) so this resolution agrees, by construction,
/// with the one `smelt_runtime::diagnostics` uses to build each cell's
/// property-profile `contract_point`.
use smelt_logical::maintenance::cell_trigger_address;

/// Find `bare_name`'s [`SourceInfo`] among `source_infos` — same bare-name
/// convention `smelt_runtime::execute::build_maint_source_facts` uses
/// (strip a leading `sources` address segment).
pub fn find_source_info<'a>(
    source_infos: &'a [SourceInfo],
    bare_name: &str,
) -> Option<&'a SourceInfo> {
    source_infos.iter().find(|info| {
        let bare = match info.address_segments.split_first() {
            Some((first, rest)) if first == "sources" => rest.join("."),
            _ => info.address_segments.join("."),
        };
        bare == bare_name
    })
}

/// Build the plain-text `smelt explain <model>` maintenance-plan report
/// (`incremental_models.md` §Surface "CLI": "prints the plan (cells, clamps,
/// locality, guarantee ledger, edges)"). Pure string-builder — no I/O — so it
/// is directly unit-testable; the caller only `println!`s the result.
///
/// `result` is the plan derived by `smelt_db::maintenance_plan_report`
/// (already-derived data; this function never re-derives admission,
/// locality, or ledger logic). `own_contract` is this model's own Relation
/// Contract fill (`docs/specs/models.md` §"The Relation Contract");
/// `edges` are its inbound edges — a declared source or an upstream
/// maintained model, rendered through the same contract rows regardless
/// of which provider filled them. `model_name` is its canonical path.
/// `cells_cfg` is the model's own `maintenance.cells[]` frontmatter (empty
/// when the model declares none) — read to look up each cell's active
/// `write:` pin, if any (`docs/specs/incremental_models.md` §"Per-cell
/// write addressing"), and its narrowed `prefer`/`technique` override for
/// the write-variant row below. `defaults_cfg` is the model's own
/// `maintenance.defaults` block (the broad end of the same ladder,
/// `smelt_logical::maintenance::choice::effective_override`).
/// `source_infos` is the project's discovered source declarations
/// (`smelt_core::discover_source_infos`) — consulted only to build a repair
/// cell's (`Technique::PerGroupRecompute`) trigger source's `SourceFacts`
/// via the single-owner `smelt_db::queries::maintenance::source_facts`, for
/// the repair stanza's affected-key discovery mechanism line; every other
/// section of this report reads neither this parameter nor a source's
/// mutation profile.
/// `edge_delta_types` is this model's already-derived per-inbound-edge
/// output-delta verdict (`docs/specs/incremental_models.md` §Surface "CLI"),
/// keyed by the same `name` an [`InboundEdgeContract`] carries — never
/// re-derived here; an edge absent from this slice prints no `delta type:`
/// row rather than a fabricated one.
/// `contract_cfg` is the model's own `contract:` frontmatter (`None` when it
/// declares none) — resolved per cell through the single-owner
/// `smelt_logical::contract::effective_contract`
/// (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 7), never
/// re-derived locally.
#[allow(clippy::too_many_arguments)]
pub fn build_maintenance_plan_report(
    model_name: &str,
    result: &smelt_db::queries::maintenance::MaintenancePlanResult,
    own_contract: &RelationContractView,
    edges: &[InboundEdgeContract],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
    defaults_cfg: Option<&smelt_core::config::MaintenanceDefaults>,
    contract_cfg: Option<&smelt_core::config::ContractConfig>,
    source_infos: &[SourceInfo],
    probes: &[smelt_runtime::probe_plan::ProbePlanEntry],
    cadence: smelt_core::config::ProbeCadence,
    edge_delta_types: &[(String, smelt_logical::analysis::output_delta::OutputDelta)],
    pending_definition_delta: Option<&(MigrationVerdict, String)>,
    own_output_delta: Option<&smelt_logical::analysis::output_delta::OutputDelta>,
) -> Result<String> {
    use smelt_logical::maintenance::PartitionLocal;
    use std::fmt::Write as _;

    let mut out = String::new();
    // Headline (`incremental_models.md` §Surface "CLI"): the model's own
    // delta signature, first line of the report — ahead of the pending
    // definition-delta note, since it is a fact about the model itself,
    // not about the run.
    let headline = delta_signature_headline(
        own_output_delta,
        result.plan.key_locality.as_ref(),
        own_contract,
    );
    let _ = writeln!(out, "model {}  {}", model_name, headline.render_text());
    let _ = writeln!(out);
    let _ = writeln!(out, "Maintenance plan: {}", model_name);
    let _ = writeln!(out);

    // Pending definition delta (`docs/specs/definition_deltas.md`
    // §"Detection"): reported ahead of a run, without deriving or executing
    // anything beyond the plan derivation itself.
    if let Some((verdict, plan_hash)) = pending_definition_delta {
        let _ = writeln!(
            out,
            "Definition delta: PENDING (verdict: {:?}, plan hash {}) — review with `smelt \
             migrate {}`, then `--apply`, or run with `--full-refresh`.",
            verdict, plan_hash, model_name
        );
        let _ = writeln!(out);
    }

    // Whole-model collapse: a column's provenance couldn't be resolved and
    // the derivation fell back to the whole-model group. `degenerate` is the
    // authoritative signal for this — non-empty exactly when
    // `grouping::derive_column_groups` had to give up on per-column
    // provenance (`maintenance_grouping.rs::degenerate_collapse_is_surfaced`).
    // `column_groups.len() == 1` alone is not a reliable proxy: a
    // legitimately single-group model spanning 2+ mutable sources is not
    // degenerate, and a genuine collapse against a single-source model still
    // has exactly one group with one source in `mutation_sensitivity`.
    if !result.degenerate.is_empty() {
        let _ = writeln!(
            out,
            "Note: {} column(s) could not distinguish per-column provenance and \
             collapsed to a single column group — this model's maintenance plan \
             treats those columns as mutation-sensitive to every listed source:",
            result.degenerate.len(),
        );
        for d in &result.degenerate {
            let _ = writeln!(out, "  - {}: {}", d.column, d.reason);
        }
        let _ = writeln!(out);
    }

    if result.plan.cells.is_empty() {
        let _ = writeln!(out, "Cells: (none)");
    } else {
        let _ = writeln!(out, "Cells ({}):", result.plan.cells.len());
        for cell in &result.plan.cells {
            let _ = writeln!(
                out,
                "  - group {} on trigger {:?}",
                cell.group, cell.trigger
            );
            let _ = writeln!(out, "      corner:    {:?}", cell.corner);
            let _ = writeln!(out, "      technique: {:?}", cell.technique);
            // Recorded availability downgrade (`state.md` §"The degradation
            // contract" step 2, `docs/outcomes/20260904-state-residency/
            // outcome.md` phase 6): omitted entirely when the cell was not
            // downgraded, matching the existing `contract_point` posture.
            if let Some(downgrade) = &cell.state_downgrade {
                let _ = writeln!(
                    out,
                    "      state downgrade: {:?} → {:?} (missing: {}) — {}",
                    downgrade.original,
                    cell.technique,
                    downgrade.missing.as_str(),
                    downgrade.reason
                );
            }
            let _ = writeln!(out, "      ledger_catch_up: {}", cell.ledger_catch_up);
            // Effective contract (`docs/specs/incremental_models.md` §"The
            // contract lattice"): default or a relaxed point, with its
            // declared parameters — resolved by the single-owner
            // `smelt_logical::contract::effective_contract`, never a local
            // model-vs-cell ladder over `ContractConfig`.
            let contract_group_columns: Vec<String> = result
                .column_groups
                .iter()
                .find(|g| g.name() == cell.group)
                .map(|g| g.columns.clone())
                .unwrap_or_default();
            let effective_contract = smelt_logical::contract::effective_contract(
                contract_cfg,
                cell_trigger_address(&cell.trigger).as_deref().unwrap_or(""),
                &contract_group_columns,
            );
            let _ = writeln!(
                out,
                "      contract:  {}",
                effective_contract.render_label()
            );
            // Region row identity (P2, `model_properties.md` §"Region row
            // identity") — plain data carried on the cell
            // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
            // Phase C3), reported alongside the technique it will drive a
            // future conditional write's compare-join for.
            let _ = writeln!(out, "      region key: {:?}", cell.row_identity.identity);
            if let Some(proven) = &cell.row_identity.proven_mismatch {
                let _ = writeln!(
                    out,
                    "      region key: NOTE declared key wins over a differing proven grain \
                     key {proven:?}"
                );
            }
            match &cell.partition_local {
                PartitionLocal::Yes => {
                    let _ = writeln!(out, "      locality:  partition_local");
                }
                PartitionLocal::No { source, why } => {
                    let _ = writeln!(
                        out,
                        "      locality:  NOT partition_local (source: {}, why: {})",
                        source, why
                    );
                }
            }
            if cell.scans.is_empty() {
                let _ = writeln!(out, "      scan clamps: (none)");
            } else {
                let _ = writeln!(out, "      scan clamps:");
                for scan in &cell.scans {
                    let _ = writeln!(
                        out,
                        "        - source={} column={} before={:?} after={:?}",
                        scan.source, scan.column, scan.before, scan.after
                    );
                }
            }
            // Open write-pattern registry (`docs/specs/incremental_models.md`
            // §"Per-cell write addressing"): the admissible pattern-name set
            // for this cell's own declared facts (structural + registry-
            // capability factors only — the fourth, backend-capability
            // factor is not narrowed here since `smelt explain` has no live
            // target connection; `BackendWriteCapabilities::all()` reports
            // every pattern a real backend *could* provide), and the active
            // `write:` pin (if any) this cell's `maintenance.cells[]` entry
            // names.
            let facts = smelt_logical::maintenance::OutputContractFacts {
                has_identity: matches!(
                    cell.row_identity.identity,
                    smelt_logical::maintenance::RowIdentity::Key(_)
                ),
                has_partition_axis: own_contract.clock.is_some(),
            };
            let admissible = smelt_logical::maintenance::admissible_write_patterns(
                facts,
                smelt_logical::maintenance::BackendWriteCapabilities::all(),
            );
            let _ = writeln!(
                out,
                "      admissible write patterns: {}",
                if admissible.is_empty() {
                    "(none)".to_string()
                } else {
                    admissible.join(", ")
                }
            );
            let write_pin_name = smelt_db::queries::maintenance::matching_write_pin(
                cell,
                &result.column_groups,
                cells_cfg,
            );
            match &write_pin_name {
                Some(pin) => {
                    let _ = writeln!(out, "      write pin: {pin}");
                }
                None => {
                    let _ = writeln!(out, "      write pin: (none)");
                }
            }
            // Observed-delta recording (`incremental_models.md` §"The graph
            // layer" — "Observed deltas on model edges"; §Known
            // Divergences): recording is wired for exactly the
            // change-suppressed column-scoped MERGE family
            // (`Technique::ColumnScopedMerge`) — the keyed-fold and
            // staged-candidate write families do not record yet, and every
            // other technique (region rewrite, in-place update) has no
            // conditional write to suppress in the first place. This reads
            // the cell's own derived `technique` only — no re-derivation of
            // whether a write is actually conditional at runtime.
            //
            // A `ColumnScopedMerge` cell only actually records at runtime
            // when `choice::resolve_write_suppression` resolves `Suppressed`
            // rather than fail-closed `Unconditional`
            // (`smelt-runtime::maintenance_driver::
            // execute_column_scoped_write_with_observed_delta`) — and P2 row
            // identity (`RowIdentity::WholeRow` never proves a per-row join
            // identity to compare on) is one of that resolution's two
            // independent fail-closed gates, alongside P3 per-column
            // comparability. `facts.has_identity` above already carries the
            // P2 half of that verdict (consulted, not re-derived); the P3
            // comparability half is not independently re-checked here — this
            // reporting path has no `sql`/`JoinContext` threaded to redo the
            // property-composition walk, so a `Key`-identity cell with an
            // incomparable compared column can still print "yes" even though
            // it resolves `Unconditional` at runtime (the authoritative
            // check remains `choice::resolve_write_suppression`).
            if cell.technique == Technique::ColumnScopedMerge && facts.has_identity {
                let _ = writeln!(
                    out,
                    "      observed-delta recording: yes (change-suppressed column-scoped MERGE)"
                );
            } else if cell.technique == Technique::ColumnScopedMerge {
                let _ = writeln!(
                    out,
                    "      observed-delta recording: no (no proven row identity — matched arm \
                     falls back to unconditional rewrite, nothing to record)"
                );
            }
            // Write variant (`incremental_models.md` §"Windowed maintenance
            // and the horizon" category 2, §"Interchangeability and
            // choice"): which matched-arm shape the override ladder's
            // conditional-variant dimension resolves for a suppressible
            // cell (`Technique::ColumnScopedMerge` or `Technique::KeyedFold`),
            // and why — pin / preference / default / first-build, mirroring
            // `choice::VariantReason`. `result.comparability` (the same
            // derivation `MaintenancePlanResult::comparability`'s own doc
            // comment names) is threaded straight into the real
            // `choice::resolve_write_suppression`/`choice::
            // resolve_write_variant` calls below — the authoritative P2/P3
            // proof, not a `facts.has_identity`-only proxy — so a
            // `technique: suppress` pin over an incomparable compared
            // column refuses here exactly as it does at runtime, not only
            // the decidable `WholeRow` (P2) case.
            if matches!(
                cell.technique,
                Technique::ColumnScopedMerge | Technique::KeyedFold
            ) {
                // The write-suppression dimension's own narrowed override
                // (`smelt_logical::maintenance::choice::effective_override`),
                // matched the same way `matching_write_pin` matches a
                // `cells[].write` pin: `on:` against this trigger's own
                // source address, `columns` against any member of the
                // cell's group. `Trigger::ColumnAdded` (the
                // definition-change trigger) has no `on:` address of its
                // own, mirroring `smelt-db`'s own `trigger_on_address`.
                let trigger_address = match &cell.trigger {
                    smelt_logical::maintenance::Trigger::NewData { source }
                    | smelt_logical::maintenance::Trigger::UpstreamMutation { source } => {
                        Some(source.clone())
                    }
                    smelt_logical::maintenance::Trigger::Backfill => Some("backfill".to_string()),
                    smelt_logical::maintenance::Trigger::ColumnAdded { .. } => None,
                };
                let group_columns: Vec<String> = result
                    .column_groups
                    .iter()
                    .find(|g| g.name() == cell.group)
                    .map(|g| g.columns.clone())
                    .unwrap_or_default();
                let overrides = trigger_address
                    .as_deref()
                    .map(|addr| {
                        smelt_logical::maintenance::choice::effective_override(
                            defaults_cfg,
                            cells_cfg,
                            addr,
                            &group_columns,
                        )
                    })
                    .unwrap_or_default();

                use smelt_core::config::{CellTechnique, TechniquePreference};

                // The real P2/P3 proof (`choice::resolve_write_suppression`)
                // over this cell's own group columns and the plan's derived
                // comparability — `WholeRow` cells short-circuit to
                // `Unconditional` inside this same call (no special-casing
                // needed here), and an incomparable compared column over a
                // proven key does too. `resolve_write_variant` folds in the
                // override ladder and is the single point a hard
                // `technique: suppress` pin over either failure propagates
                // as a real `explain` error, never a silently-wrong success
                // line.
                let raw_suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
                    &group_columns,
                    &result.comparability,
                    &cell.row_identity,
                );
                smelt_logical::maintenance::choice::resolve_write_variant(
                    &raw_suppression,
                    &cell.trigger,
                    cell.ledger_catch_up,
                    &overrides,
                )
                .map_err(|refusal| anyhow::anyhow!("{refusal}"))?;

                if let smelt_logical::maintenance::choice::WriteSuppression::Unconditional { why } =
                    &raw_suppression
                {
                    let _ = writeln!(
                        out,
                        "      write variant: unconditional (not admitted — {why})"
                    );
                } else if let Some(CellTechnique::Suppress) = overrides.technique {
                    let _ = writeln!(
                        out,
                        "      write variant: suppressed (pinned via `technique: suppress`)"
                    );
                } else if let Some(CellTechnique::Unconditional) = overrides.technique {
                    let _ = writeln!(
                        out,
                        "      write variant: unconditional (pinned via `technique: \
                         unconditional`, overriding whatever the structural default would \
                         otherwise prefer)"
                    );
                } else if matches!(overrides.prefer, Some(TechniquePreference::Suppress)) {
                    let _ = writeln!(
                        out,
                        "      write variant: suppressed (soft-preferred via `prefer: \
                         suppress`, overriding the structural default)"
                    );
                } else if matches!(overrides.prefer, Some(TechniquePreference::Unconditional)) {
                    let _ = writeln!(
                        out,
                        "      write variant: unconditional (soft-preferred via `prefer: \
                         unconditional`, overriding the structural default)"
                    );
                } else if smelt_logical::maintenance::choice::trigger_has_prior_state(
                    &cell.trigger,
                    cell.ledger_catch_up,
                ) {
                    let _ = writeln!(
                        out,
                        "      write variant: suppressed (preference — steady-state trigger \
                         over prior state)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      write variant: unconditional (first-build posture — this trigger \
                         has no prior stored state on this column group to diff against; the \
                         conditional variant is admitted but not preferred here)"
                    );
                }
            }

            // Repair stanza (`docs/specs/incremental_models.md` §"The repair
            // family"): technique-scoped to `Technique::PerGroupRecompute`
            // only, so every non-repair cell's rendering above is
            // byte-identical to before this stanza existed.
            if cell.technique == Technique::PerGroupRecompute {
                match &cell.row_identity.identity {
                    smelt_logical::maintenance::RowIdentity::Key(cols) => {
                        let _ = writeln!(
                            out,
                            "      repair key slice: {} (sound over-approximation)",
                            cols.join(", ")
                        );
                    }
                    smelt_logical::maintenance::RowIdentity::WholeRow => {
                        let _ = writeln!(out, "      repair key slice: (not derived)");
                    }
                }
                let trigger_source = match &cell.trigger {
                    smelt_logical::maintenance::Trigger::NewData { source }
                    | smelt_logical::maintenance::Trigger::UpstreamMutation { source } => {
                        Some(source.clone())
                    }
                    smelt_logical::maintenance::Trigger::Backfill
                    | smelt_logical::maintenance::Trigger::ColumnAdded { .. } => None,
                };
                match trigger_source
                    .as_deref()
                    .and_then(|src| cell.scans.iter().find(|c| c.source == src))
                    .or_else(|| cell.scans.first())
                {
                    Some(scan) => {
                        let _ = writeln!(
                            out,
                            "      repair read bound: source={} column={} before={:?} after={:?}",
                            scan.source, scan.column, scan.before, scan.after
                        );
                    }
                    None => {
                        let _ = writeln!(out, "      repair read bound: (not derived)");
                    }
                }
                // Key-addressed model-edge cell (`incremental_models.md`
                // §"Upstream model edges"): `cell.key_scope` names the third
                // discovery posture — the group-grain fingerprint-sidecar
                // diff over the upstream's own output table
                // (`smelt_runtime::maintenance_driver::
                // resolve_key_addressed_affected_keys`), not a declared
                // source's mutation profile, so `find_source_info` (which
                // only resolves a declared `sources.*` name) is never
                // consulted for this branch.
                if let Some(scope) = &cell.key_scope {
                    let route = match scope.discovery {
                        smelt_logical::maintenance::KeyDiscovery::UpstreamKeyed => {
                            "keyed at the upstream's own KeyedUpsert key columns"
                        }
                        smelt_logical::maintenance::KeyDiscovery::DownstreamGrainOverUpstream => {
                            "keyed at the downstream's own grain, projected over the upstream \
                             relation"
                        }
                    };
                    let _ = writeln!(
                        out,
                        "      affected-key discovery: group-grain fingerprint-sidecar diff \
                         over the upstream's own output table ({route})"
                    );
                } else {
                    match trigger_source
                        .as_deref()
                        .and_then(|src| find_source_info(source_infos, src))
                    {
                        Some(info) => {
                            let src_facts = smelt_db::queries::maintenance::source_facts(
                                trigger_source.as_deref().unwrap_or_default(),
                                Some(info),
                                false,
                            );
                            let mechanism = match discovery_posture(src_facts.mutation) {
                                Some(RepairDiscoveryPosture::ClampedScan) => {
                                    "clamped current-source scan"
                                }
                                Some(RepairDiscoveryPosture::SidecarDiff) => {
                                    "group-grain fingerprint-sidecar diff (mutable_snapshot, \
                                     obligation 7)"
                                }
                                // A repair cell is never admitted over a
                                // change_feed source (`derive::
                                // derive_new_data` refuses it upstream), so
                                // this arm should be unreachable for any
                                // cell `smelt explain` actually renders.
                                None => "(no discovery posture — unexpected change_feed source)",
                            };
                            let _ = writeln!(out, "      affected-key discovery: {mechanism}");
                        }
                        None => {
                            let _ = writeln!(out, "      affected-key discovery: (not derived)");
                        }
                    }
                }

                // `write: diff_patch` resolution: the real
                // `choice::resolve_cell_choice`, never a display-only
                // re-derivation. Only prints a line when the write pin
                // actually resolves this cell to `ChosenTechnique::DiffPatch`
                // — a cell with no pin, or a pin resolving to something else,
                // prints nothing further.
                let repair_trigger_address = match &cell.trigger {
                    smelt_logical::maintenance::Trigger::NewData { source }
                    | smelt_logical::maintenance::Trigger::UpstreamMutation { source } => {
                        Some(source.clone())
                    }
                    smelt_logical::maintenance::Trigger::Backfill => Some("backfill".to_string()),
                    smelt_logical::maintenance::Trigger::ColumnAdded { .. } => None,
                };
                let repair_group_columns: Vec<String> = result
                    .column_groups
                    .iter()
                    .find(|g| g.name() == cell.group)
                    .map(|g| g.columns.clone())
                    .unwrap_or_default();
                let repair_overrides = repair_trigger_address
                    .as_deref()
                    .map(|addr| {
                        smelt_logical::maintenance::choice::effective_override(
                            defaults_cfg,
                            cells_cfg,
                            addr,
                            &repair_group_columns,
                        )
                    })
                    .unwrap_or_default();
                let write_pattern = write_pin_name.as_deref().and_then(lookup_write_pattern);
                let chosen = resolve_cell_choice(
                    Some(cell),
                    &cell.trigger,
                    &repair_overrides,
                    write_pattern,
                    false,
                )
                .map_err(|refusal| anyhow::anyhow!("{refusal}"))?;
                if let ChosenTechnique::DiffPatch {
                    recompute: Technique::PerGroupRecompute,
                    delete_leg,
                } = chosen
                {
                    let _ = writeln!(out, "      write mechanism: diff_patch");
                    let delete_leg_line = match delete_leg {
                        DeleteLeg::Complete => "complete".to_string(),
                        DeleteLeg::Omitted { why } => format!("omitted ({why})"),
                    };
                    let _ = writeln!(out, "      diff_patch delete leg: {delete_leg_line}");
                }
            }
        }
    }
    let _ = writeln!(out);

    // Derived execution postures (`incremental_shapes.md` §"Derived
    // execution postures", `docs/outcomes/20260815-keyed-grain-residue`
    // phase 4): read straight off `result.execution_postures` — this
    // function derives nothing, `smelt_logical::execution_postures` is the
    // single owner. `None` for a model that never classified as `grain:
    // key` (nothing to derive postures over); omitted entirely rather than
    // printed as a false negative.
    if let Some(postures) = &result.execution_postures {
        let run_shape = match result.is_snapshot_reconcile {
            Some(true) => "snapshot-reconcile",
            Some(false) => "window-forward",
            None => "n/a",
        };
        let _ = writeln!(out, "Execution postures:");
        let _ = writeln!(out, "  run shape: {run_shape}");
        let _ = writeln!(
            out,
            "  re-run tolerance: {} ({})",
            if postures.rerun_tolerant.holds {
                "yes"
            } else {
                "no"
            },
            postures.rerun_tolerant.reason
        );
        let _ = writeln!(
            out,
            "  order-independence: {} ({})",
            if postures.order_independent.holds {
                "yes"
            } else {
                "no"
            },
            postures.order_independent.reason
        );
        let _ = writeln!(
            out,
            "  reprocessing: refused ({})",
            postures.reprocessing_refused.reason
        );
        let _ = writeln!(out);
    }

    // Internal state columns (`incremental_shapes.md` §"Decomposed state
    // (rung 2) in keyed models", `docs/outcomes/20260809-rung2-state-shapes`
    // row 9): one entry per presented column that folds through hidden
    // decomposed state, read straight off `result.state_columns` — this
    // function derives nothing, `classify_cumulative` is the single owner
    // of which columns are state-bearing. Omitted entirely (no empty
    // header) for a model with no state-bearing columns.
    if !result.state_columns.is_empty() {
        let _ = writeln!(out, "State columns:");
        for summary in &result.state_columns {
            let _ = writeln!(
                out,
                "  - {} (presented) folds through: {}",
                summary.presented_column,
                summary.state_columns.join(", ")
            );
            let _ = writeln!(out, "      presentation: {}", summary.presentation_expr);
            let _ = writeln!(out, "      not part of the model's public schema");
        }
        let _ = writeln!(out);
    }

    // Key temporal locality (`incremental_shapes.md` §"Key temporal
    // locality (the time-partitioned output)"): for an admitted `grain:
    // key` + `timeseries:` model, print the established route/slice and
    // the derived settle bound. Route 2's settle bound is honestly `Never`
    // via `SettleBound`'s own `Debug` — never a large sentinel duration
    // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase A5's review checklist).
    if let Some(locality) = &result.plan.key_locality {
        use smelt_logical::maintenance::locality::LocalitySlice;
        let route = match &locality.slice {
            LocalitySlice::Window {
                recurrence_bounded: false,
                ..
            } => "route 1 (key-embedded)",
            LocalitySlice::Window {
                recurrence_bounded: true,
                ..
            } => "route 3 (recurrence-bounded, statically derived)",
            LocalitySlice::DeltaValues { .. } => "route 2 (key-determined)",
            LocalitySlice::RecurrenceBounded { .. } => {
                "route 3 (recurrence-bounded, declared key_recurrence)"
            }
        };
        // Observed-delta key→partition projection form
        // (`incremental_shapes.md` §"What the composed shape uniquely
        // enables" — "Exact key→partition dirt projection"; §Known
        // Divergences): routes 1–2 project a recorded observed delta to
        // *exact* touched partitions (a stored row's partition value is a
        // per-key constant); route 3 widens the projection backward by the
        // recurrence bound `r` plus the route's own margins, since a key's
        // partition value may move under that route. This mirrors
        // `smelt_logical::maintenance::propagate::project_observed_delta`'s
        // own route dispatch — read here, never re-derived.
        let projection = match &locality.slice {
            LocalitySlice::Window {
                recurrence_bounded: false,
                ..
            } => "exact (key-embedded)",
            LocalitySlice::DeltaValues { .. } => "exact (key-determined)",
            LocalitySlice::Window {
                recurrence_bounded: true,
                ..
            }
            | LocalitySlice::RecurrenceBounded { .. } => "widened by `r` + margins",
        };
        let _ = writeln!(out, "Key temporal locality:");
        let _ = writeln!(out, "  route: {route}");
        let _ = writeln!(out, "  slice: {:?}", locality.slice);
        let _ = writeln!(out, "  settle bound: {:?}", locality.settle_bound);
        let _ = writeln!(out, "  observed-delta projection: {projection}");
        let _ = writeln!(out);
    }

    if result.plan.refusals.is_empty() {
        let _ = writeln!(out, "Refusals: (none)");
    } else {
        let _ = writeln!(out, "Refusals ({}):", result.plan.refusals.len());
        for refusal in &result.plan.refusals {
            let _ = writeln!(out, "  - {:?}", refusal);
        }
    }
    let _ = writeln!(out);

    // Relation Contract (`docs/specs/models.md` §"The Relation Contract"):
    // this model's own clock/identity/derived-grain rows, then one contract
    // block per inbound edge — a source and an upstream model render
    // through the same rows (`write_relation_contract`), never a
    // provider-specific format.
    let _ = writeln!(out, "Relation contract:");
    write_relation_contract(&mut out, "  ", own_contract);
    let _ = writeln!(out);

    if edges.is_empty() {
        let _ = writeln!(out, "Inbound edges: (none)");
    } else {
        let names: Vec<&str> = edges.iter().map(|e| e.name.as_str()).collect();
        let _ = writeln!(out, "Inbound edges: {}", names.join(", "));
        for edge in edges {
            let provider = match edge.provider {
                RelationContractProvider::Source => "source",
                RelationContractProvider::Model => "model",
            };
            let _ = writeln!(out, "  - {} ({})", edge.name, provider);
            write_relation_contract(&mut out, "      ", &edge.contract);
            if let Some((_, shape)) = edge_delta_types.iter().find(|(name, _)| name == &edge.name) {
                let _ = writeln!(out, "      delta type: {}", format_output_delta(shape));
            }
        }
    }
    let _ = writeln!(out);

    // Declared-fact probes (`docs/specs/model_properties.md` §"Probe
    // obligation", `docs/specs/cli.md` §"`smelt explain <model>`
    // maintenance-plan report"): the fact, its named diagnostic, the cell
    // it licenses, and its static per-run cost — read verbatim from the
    // shared `smelt_runtime::probe_plan` builder, never re-derived here.
    // The project cadence line applies to every listed probe; a run under
    // `probes: {cadence: off}` still lists them (trusted, not verified).
    if probes.is_empty() {
        let _ = writeln!(out, "Probes (0):");
    } else {
        let _ = writeln!(out, "Probes ({}):", probes.len());
        let _ = writeln!(out, "  cadence: {}", format_probe_cadence(cadence));
        for probe in probes {
            let _ = writeln!(out, "  - fact: {}", probe.fact);
            let _ = writeln!(out, "      probe: {}", probe.probe);
            let _ = writeln!(out, "      licensed cell: {}", probe.cell);
            let _ = writeln!(out, "      cost: {}", probe.cost);
        }
    }

    Ok(out)
}

/// The project `probes:` cadence rendered as one line, shared by the text
/// report and the JSON `cadence` field's source of truth
/// (`docs/specs/smelt_yml.md` §"Top-level keys" `probes:`).
fn format_probe_cadence(cadence: smelt_core::config::ProbeCadence) -> String {
    match cadence {
        smelt_core::config::ProbeCadence::PerRun => "per_run".to_string(),
        smelt_core::config::ProbeCadence::Periodic { every_n_runs } => {
            format!("periodic (every {every_n_runs} runs)")
        }
        smelt_core::config::ProbeCadence::Off => "off".to_string(),
    }
}

/// How a `--show-sql` region's literal bounds are sourced
/// (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
/// report"): real values from `--period <start>..<end>`, or the symbolic
/// placeholders `{{window_start}}`/`{{window_end}}` when no period is
/// given, so the emitted shape is inspectable without choosing a window.
#[derive(Debug, Clone)]
pub enum RegionLiterals {
    Period { start: String, end: String },
    Placeholders,
}

/// The `--period`-derived output window and per-source scan margin a
/// `Technique::DeleteInsert` cell's statements are built from — computed once
/// per `--show-sql` invocation (in `commands/explain.rs`, the only caller)
/// via `smelt_runtime::windowing::compute_incremental_windows`, the same
/// single-owner skew-inversion derivation a live run's `build_model_plans`
/// uses (`docs/specs/model_transforms.md` §Semantics "The output window is
/// derived, never assumed"). `None` when no `--period` was given (the
/// symbolic-placeholder report), in which case the DeleteInsert branch falls
/// back to the placeholder literals with no derivation (there is no concrete
/// date to derive a skew inversion from).
pub struct DerivedWindow {
    /// The derived output window's inclusive start (`%Y-%m-%d`) — identity to
    /// the requested `--period` start for an identity `partition_column`,
    /// skew-inverted (earlier) otherwise.
    pub output_start: String,
    /// The derived output window's exclusive end (`%Y-%m-%d`).
    pub output_end: String,
    /// Per-upstream-source scan margin (`smelt_runtime::build_model_source_bounds`),
    /// the input `derive_batch_filtered_sql` widens the read by, relative to
    /// the output window above — the two-layer widened-scan
    /// (`docs/specs/model_transforms.md` §Semantics "Source-filter pushdown +
    /// the two clamps").
    pub scan_bounds: std::collections::HashMap<String, SourceBound>,
    /// The model's own derived partition-column skew bound
    /// (`IncrementalWindows::skew`) — reused, never re-derived, by
    /// `derive_batch_filtered_sql`'s transparent-slice fast-path gate.
    pub skew: smelt_logical::analysis::source_bounds::Skew,
    /// The clock-pin literal `derive_batch_filtered_sql` freezes
    /// run-deterministic calls (`NOW()`, …) to. `--show-sql` has no real run
    /// clock, so this is simply the report's own build time.
    pub run_start: chrono::DateTime<chrono::Utc>,
    /// The axis `output_start`/`output_end` are rendered in — inferred by
    /// `commands/explain.rs::build_derived_window` from the `--period`
    /// literal's own form (no schema handle to resolve it from directly).
    pub axis: smelt_logical::PartitionAxis,
}

/// One cell's `--show-sql` statement report: the cell it belongs to
/// (by index into `MaintenancePlanResult.plan.cells`) and either the
/// emitted [`StatementGroup`] or a plain-language reason none could be
/// built (`Technique::InPlaceUpdate` has no production consumer yet, or
/// the cell's technique-specific inputs — e.g. a keyed-fold driving source
/// — could not be classified from the discovered project).
pub struct CellStatements {
    pub cell_index: usize,
    pub outcome: Result<StatementGroup, String>,
}

/// Render `diag_cell`'s own `Admitted` technique-preview entry — the single
/// shared deriver of a cell's statement shape
/// (`smelt_runtime::diagnostics::build_model_diagnostics`,
/// `docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer
/// boundary") — as a [`StatementGroup`], substituting the real `--period`
/// literals for the builder's own symbolic `{{window_start}}`/
/// `{{window_end}}` placeholders when a concrete period was given.
///
/// This plain token substitution is exact (not an approximation) for every
/// technique except `Technique::DeleteInsert` under a concrete `--period`:
/// the shared builder's placeholder statements carry the two tokens
/// verbatim wherever a literal bound would otherwise appear —
/// `inject_time_filter`/`inject_source_filters` both pass a zero-margin
/// bound through unchanged (`subtract_seconds_from_date`/
/// `add_seconds_to_date`'s `secs == 0` fast path returns the input
/// untouched), never reformatting or recomputing it — so swapping the token
/// text for the real literal reproduces byte-for-byte what re-deriving the
/// statement with the real literal in hand would have produced.
/// `Technique::DeleteInsert` under a real `--period` is the one case this
/// cannot cover: see [`build_delete_insert_period_statement_group`]'s own
/// doc comment for why.
pub fn build_admitted_statement_group(
    diag_cell: &PlanCellDiagnostics,
    region: &RegionLiterals,
) -> Result<StatementGroup, String> {
    let preview = diag_cell
        .technique_previews
        .iter()
        .find(|p| matches!(p.admissibility, Admissibility::Admitted))
        .ok_or_else(|| {
            "no Admitted technique preview entry for this cell — the shared diagnostics \
             builder always populates exactly one (docs/specs/ui_model_diagnostics.md \
             §Semantics \"Admissibility verdict\")"
                .to_string()
        })?;

    if preview.statements.is_empty() {
        // NotApplicable/empty preview: the builder could not render this
        // technique at all for this cell — surface the same reason, never a
        // silently empty success.
        return match &preview.admissibility {
            Admissibility::NotApplicable { reason } => Err(reason.clone()),
            _ => Err("the admitted technique preview has no statements".to_string()),
        };
    }

    let statements = preview
        .statements
        .iter()
        .map(|s| smelt_logical::maintenance::emit::MaintenanceStatement {
            sql: match region {
                RegionLiterals::Period { start, end } => s
                    .sql
                    .replace("{{window_start}}", start)
                    .replace("{{window_end}}", end),
                RegionLiterals::Placeholders => s.sql.clone(),
            },
        })
        .collect();

    Ok(StatementGroup {
        statements,
        transactional: preview.transactional,
    })
}

/// Build `cell`'s `Technique::DeleteInsert` statement group for a concrete
/// `--period`, deriving the real output window and per-source scan margin
/// the same way a live run's `build_model_plans` would
/// (`smelt_runtime::derive_batch_filtered_sql`,
/// `docs/specs/model_transforms.md` §Semantics "The output window is
/// derived, never assumed").
///
/// This is the one `--show-sql` statement shape
/// [`build_admitted_statement_group`]'s token substitution cannot
/// reproduce: the real output window can differ from the literal `--period`
/// text (skew inversion, e.g. `silver.sessions` in `examples/web_analytics`)
/// and the real read additionally widens per-source scan pushdown that the
/// shared builder's symbolic preview never renders at all — it is
/// deliberately display-only (`smelt_runtime::diagnostics::
/// build_technique_statements`'s own doc comment: "a technique-preview
/// build is a display-only illustration of a cell's shape, not a
/// `--period`-bound dry run"). `smelt-runtime` remains the sole deriver of
/// the *symbolic* preview shape every technique-preview entry carries; this
/// function is `smelt-cli`'s own real-window dry-run rendering for the one
/// technique whose real statements a concrete `--period` changes — a
/// distinct concern from a technique preview, not a second copy of the same
/// derivation (`docs/specs/ui_model_diagnostics.md` §Semantics
/// "Thin-consumer boundary").
///
/// `resolver` must be built from the *actual* discovered project (the same
/// way `smelt-runtime`'s dry-run compile path in `execute.rs` builds it),
/// not `EphemeralResolver::empty()` — see `build_admitted_statement_group`'s
/// sibling doc note in `commands/explain.rs` for the same requirement.
#[allow(clippy::too_many_arguments)]
fn build_delete_insert_period_statement_group(
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    dw: &DerivedWindow,
) -> Result<StatementGroup, String> {
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let table_name = format!("{schema}.{}", model.db_name_owned());

    let partition_col = model
        .metadata
        .as_deref()
        .and_then(|m| m.timeseries.as_ref())
        .map(|t| t.partition_column.clone())
        .ok_or_else(|| {
            "no timeseries.partition_column declared — cannot build the region \
             DELETE+INSERT pair"
                .to_string()
        })?;

    // Derive the statements exactly as a live run's `derive_batch_filtered_sql`
    // would for this window — output clamp *and* the per-source widened-scan
    // pushdown, composed in one single-owner call rather than re-implemented
    // here. This clamps the model's own (uncompiled) SQL *before* compiling —
    // the same order the live run uses — so a model whose outermost FROM is a
    // `TableExpr`-returning function call (`smelt.functions.f(...)`) never
    // asks the parser to re-parse the compiler's own function-expansion
    // output (which is reparse-hostile even though it is DuckDB-executable).
    let run_range = TimeRange {
        start: dw.output_start.clone(),
        end: dw.output_end.clone(),
        axis: dw.axis,
    };
    let filtered_sql = smelt_runtime::derive_batch_filtered_sql(
        &stripped_sql,
        &partition_col,
        &dw.scan_bounds,
        &run_range,
        dw.run_start,
        dw.skew,
    )
    .map_err(|e| format!("failed to inject the output clamp: {e}"))?;

    let compiled = registry
        .get(target)
        .compile_with_sql_and_ephemerals(model, schema, &filtered_sql, resolver)
        .map_err(|e| format!("failed to compile model body: {e}"))?;

    let region_used = smelt_logical::maintenance::emit::Region::for_axis(
        dw.axis,
        &dw.output_start,
        &dw.output_end,
    )
    .map_err(|e| format!("failed to render the DELETE/INSERT region literal: {e}"))?;

    Ok(smelt_logical::maintenance::emit::emit_delete_insert(
        &table_name,
        &partition_col,
        &region_used,
        &compiled.sql,
        dialect,
    ))
}

/// Build a [`CellStatements`] entry for every cell in `plan`, in the same
/// order they appear in the report (`docs/specs/cli.md`: "Statements print
/// in execution order"). `diag_cells` is `ModelDiagnostics::cells`, in the
/// same cell order as `plan_cells` — every cell but a `Technique::
/// DeleteInsert` cell under a concrete `--period` reads its statements
/// straight from `diag_cells[i]`'s own `Admitted` preview
/// ([`build_admitted_statement_group`]); only that one combination re-derives
/// with the real window ([`build_delete_insert_period_statement_group`]).
#[allow(clippy::too_many_arguments)]
pub fn build_all_cell_statements(
    plan_cells: &[PlanCell],
    diag_cells: &[PlanCellDiagnostics],
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    region: &RegionLiterals,
    derived: Option<&DerivedWindow>,
) -> Vec<CellStatements> {
    plan_cells
        .iter()
        .zip(diag_cells.iter())
        .enumerate()
        .map(|(cell_index, (cell, diag_cell))| {
            let outcome = match (cell.technique, derived) {
                (Technique::DeleteInsert, Some(dw)) => build_delete_insert_period_statement_group(
                    model, schema, target, registry, resolver, dialect, dw,
                ),
                _ => build_admitted_statement_group(diag_cell, region),
            };
            CellStatements {
                cell_index,
                outcome,
            }
        })
        .collect()
}

/// Render `statements` as the plain-text block `--show-sql` appends after
/// each cell's report block: statements in execution order, a transactional
/// group bracketed by `BEGIN`/`COMMIT` lines to show its atomicity (the
/// backend supplies the real transaction mechanics at run time).
pub fn render_cell_statements_text(statements: &[CellStatements]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for cs in statements {
        let _ = writeln!(out, "  cell[{}] statements:", cs.cell_index);
        match &cs.outcome {
            Ok(group) => {
                out.push_str(&render_statement_group_text(group, "    "));
            }
            Err(reason) => {
                let _ = writeln!(out, "    (no statements: {reason})");
            }
        }
    }
    out
}

/// Render one emitted [`StatementGroup`] as the plain-text block both
/// `smelt explain <model> --show-sql` and `smelt run`/`smelt rebuild
/// --dry-run` print for a maintenance statement: a transactional group is
/// bracketed by `BEGIN`/`COMMIT` lines to show its atomicity (the backend
/// supplies the real transaction mechanics at run time), a single-statement
/// group prints its statement directly. `indent` prefixes every line so the
/// caller controls nesting (`--show-sql` nests each group under its cell;
/// `--dry-run` prints at the top level).
pub fn render_statement_group_text(group: &StatementGroup, indent: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if group.transactional {
        let _ = writeln!(out, "{indent}BEGIN");
        for stmt in &group.statements {
            let _ = writeln!(out, "{indent}  {}", stmt.sql);
        }
        let _ = writeln!(out, "{indent}COMMIT");
    } else {
        for stmt in &group.statements {
            let _ = writeln!(out, "{indent}{}", stmt.sql);
        }
    }
    out
}

/// Render `--show-sql --technique <name>` output: for every cell, the
/// requested technique's own preview entry from the shared `smelt-runtime::
/// diagnostics` builder (`docs/specs/ui_model_diagnostics.md` §Surface
/// "CLI") — its SQL statements and admissibility verdict, printed in place
/// of the admitted technique's statements [`render_cell_statements_text`]
/// prints by default. A cell where the requested technique's own verdict is
/// `NotApplicable` still prints its reason and any illustrative SQL the
/// builder could still render — never silently omitted (fail-loud
/// discipline, `CLAUDE.md` §"Fail-loud discipline").
///
/// `cells` is `ModelDiagnostics::cells`, in the same cell order `--show-sql`
/// itself reports (`build_model_diagnostics` maps 1:1 over the same
/// `MaintenancePlanResult::plan.cells` this report's own statements are
/// built from) — every requested technique is a member of the closed
/// registry `ModelDiagnostics` always populates
/// (`smelt_runtime::diagnostics::ALL_TECHNIQUES`), so every cell is
/// guaranteed to carry a matching preview entry.
pub fn render_technique_previews_text(
    cells: &[PlanCellDiagnostics],
    requested: Technique,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (cell_index, cell) in cells.iter().enumerate() {
        let _ = writeln!(out, "  cell[{cell_index}] technique preview: {requested:?}");
        match cell
            .technique_previews
            .iter()
            .find(|p| p.technique == requested)
        {
            Some(preview) => {
                let _ = writeln!(
                    out,
                    "    verdict: {}",
                    format_admissibility(&preview.admissibility)
                );
                if preview.statements.is_empty() {
                    let _ = writeln!(out, "    (no statements)");
                } else if preview.transactional {
                    let _ = writeln!(out, "    BEGIN");
                    for stmt in &preview.statements {
                        let _ = writeln!(out, "      {}", stmt.sql);
                    }
                    let _ = writeln!(out, "    COMMIT");
                } else {
                    for stmt in &preview.statements {
                        let _ = writeln!(out, "    {}", stmt.sql);
                    }
                }
            }
            None => {
                // `ALL_TECHNIQUES` always yields one preview entry per known
                // technique for every cell (`docs/specs/ui_model_diagnostics.md`
                // §Semantics "Technique preview set" — "never partial by
                // omission") — reaching here would mean the registry and
                // this CLI's accepted `--technique` names have drifted
                // apart, so this is reported rather than silently skipped.
                let _ = writeln!(
                    out,
                    "    (no preview entry: {requested:?} is not in the technique registry)"
                );
            }
        }
    }
    out
}

/// Render one [`Admissibility`] verdict as the text `--technique` prints
/// after each cell's preview header.
fn format_admissibility(a: &Admissibility) -> String {
    match a {
        Admissibility::Admitted => {
            "Admitted (the technique the plan actually resolved for this cell)".to_string()
        }
        Admissibility::InterchangeableAlternative => {
            "InterchangeableAlternative (proven sound for this cell, but not the one the plan \
             resolved)"
                .to_string()
        }
        Admissibility::NotApplicable { reason } => format!("NotApplicable — {reason}"),
    }
}

/// JSON shape for one statement in `--json --show-sql`'s per-cell
/// `statements` array (`docs/specs/cli.md`:
/// `{"sql": "<statement>", "transactional_group": <int>}`).
#[derive(Debug, Serialize, Clone)]
pub struct ExplainStatementJson {
    pub sql: String,
    pub transactional_group: usize,
}

/// JSON shape for one cell's `--json --show-sql` entry.
#[derive(Debug, Serialize)]
pub struct ExplainCellJson {
    pub group: String,
    pub trigger: String,
    pub corner: String,
    pub technique: String,
    /// The region row identity (P2, `model_properties.md` §"Region row
    /// identity"): `"Key([...])"` or `"WholeRow"`.
    pub row_identity: String,
    /// The proven grain key that a declared `unique_key` overrode, when the
    /// two disagreed while both were present — surfaced rather than
    /// silently dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_identity_proven_mismatch: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_statements_reason: Option<String>,
    pub statements: Vec<ExplainStatementJson>,
    /// The full technique-preview array for this cell — one entry per
    /// technique in the closed registry, never just the admitted one
    /// (`docs/specs/ui_model_diagnostics.md` §Surface "CLI": "`--json`
    /// gains the full technique-preview array per cell (all techniques,
    /// not just the admitted one)"). Read verbatim from the shared
    /// `smelt-runtime::diagnostics` builder — never re-derived here
    /// (§Semantics "Thin-consumer boundary").
    pub technique_previews: Vec<TechniquePreview>,
    /// This cell's effective contract lattice point — default (an empty
    /// object) or the applicable relaxations with their declared parameters
    /// (`docs/specs/incremental_models.md` §"The contract lattice"),
    /// resolved through the single-owner `smelt_logical::contract::
    /// effective_contract`. An append-stable addition to this JSON shape
    /// (`docs/specs/cli.md` §Constraints item 5).
    pub contract_point: ExplainContractPointJson,
    /// This cell's recorded availability downgrade (`state.md` §"The
    /// degradation contract" step 2), absent when the cell was not
    /// downgraded — an append-stable addition to this JSON shape
    /// (`docs/specs/cli.md` §Constraints item 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_downgrade: Option<ExplainStateDowngradeJson>,
}

/// JSON shape of one cell's recorded state downgrade (`smelt explain
/// --json`): the technique ideal derivation chose, the structure that was
/// missing, and the rendered reason (`docs/specs/state.md` §"The
/// degradation contract").
#[derive(Debug, Serialize)]
pub struct ExplainStateDowngradeJson {
    pub original: String,
    pub missing: String,
    pub reason: String,
}

/// JSON shape of one cell's effective contract (`smelt explain --json`):
/// absent relaxations are omitted, never rendered as `null`. Single-owned
/// in `smelt_logical::contract::ContractPointView`
/// (`docs/outcomes/20260905-property-diff/phases/02-plan.md` task 5) —
/// kept as a type alias here so every existing
/// `smelt_cli::explain::ExplainContractPointJson` reference keeps working
/// unchanged. Sourced from `smelt_logical::contract::effective_contract`
/// inside `smelt_runtime::diagnostics::build_model_diagnostics`, never
/// re-resolved here.
pub type ExplainContractPointJson = smelt_logical::contract::ContractPointView;

/// The `--json --show-sql` per-model report:
/// `{"model": "<name>", "contract": {...}, "inbound_edges": [...], "cells":
/// [...], "properties": {...}}`, carrying the Relation Contract slots
/// (`docs/specs/models.md` §"The Relation Contract") for both this model
/// and every inbound edge, each cell's own `statements` and
/// `technique_previews` arrays, and the model's full derived property set
/// (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
/// report"; `docs/specs/ui_model_diagnostics.md` §Surface "CLI"). Both
/// `properties` and every cell's `technique_previews` are read verbatim
/// from `smelt_runtime::diagnostics::ModelDiagnostics` — this JSON shape
/// never derives them itself.
#[derive(Debug, Serialize)]
pub struct ExplainMaintenanceJson {
    pub model: String,
    /// The model's own delta signature (`incremental_models.md` §Surface
    /// "CLI", Headline bullet) — the SAME struct the text report's first
    /// line renders, so `--json` and text can never disagree. An
    /// append-stable addition to this JSON shape (`docs/specs/cli.md`
    /// §Constraints item 5).
    pub delta_signature: DeltaSignatureHeadline,
    pub contract: RelationContractView,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inbound_edges: Vec<InboundEdgeContract>,
    pub cells: Vec<ExplainCellJson>,
    pub properties: PropertySet,
    /// One entry per presented column that folds through hidden decomposed
    /// state (`incremental_shapes.md` §"Decomposed state (rung 2) in keyed
    /// models"), empty for a model with none — an append-stable addition to
    /// this JSON shape (`docs/specs/cli.md` §Constraints item 5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub state_columns: Vec<smelt_logical::StateColumnSummary>,
    /// The model's three derived execution postures
    /// (`incremental_shapes.md` §"Derived execution postures"), absent for
    /// a model that never classified as `grain: key` — an append-stable
    /// addition to this JSON shape (`docs/specs/cli.md` §Constraints item
    /// 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_postures: Option<ExplainExecutionPosturesJson>,
    /// The model's declared-fact probe set (`docs/specs/model_properties.md`
    /// §"Probe obligation"), empty for a model declaring none — an
    /// append-stable addition to this JSON shape (`docs/specs/cli.md`
    /// §Constraints item 5).
    pub probes: Vec<ExplainProbeJson>,
    /// The model's maintenance-plan admission refusals
    /// (`docs/specs/property_diff.md` §"The property profile" item 3),
    /// read verbatim from the property profile — an append-stable addition
    /// to this JSON shape (`docs/specs/cli.md` §Constraints item 5). Empty
    /// when the model's plan admitted every cell.
    pub refusals: Vec<smelt_logical::analysis::profile::ProfileRefusal>,
    /// A pending, non-eclipsed, unapproved definition delta
    /// (`docs/specs/definition_deltas.md` §"Detection"), absent when there
    /// is none — an append-stable addition to this JSON shape
    /// (`docs/specs/cli.md` §Constraints item 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_delta: Option<ExplainDefinitionDeltaJson>,
}

/// A pending definition delta, as reported by `smelt explain --json`
/// (`docs/specs/definition_deltas.md` §"Detection").
#[derive(Debug, Serialize)]
pub struct ExplainDefinitionDeltaJson {
    pub status: String,
    pub verdict: String,
    pub plan_hash: String,
}

/// One derived execution posture's verdict, as reported by `smelt explain
/// --json` — mirrors `smelt_logical::PostureVerdict`.
#[derive(Debug, Serialize)]
pub struct ExplainPostureVerdictJson {
    pub holds: bool,
    pub reason: String,
}

/// The `execution_postures` object of `smelt explain --json`'s output
/// (`docs/specs/incremental_shapes.md` §"Derived execution postures"): the
/// derived run shape plus the three posture verdicts, read verbatim from
/// `smelt_logical::execution_postures` — never re-derived here.
#[derive(Debug, Serialize)]
pub struct ExplainExecutionPosturesJson {
    pub run_shape: String,
    pub rerun_tolerant: ExplainPostureVerdictJson,
    pub order_independent: ExplainPostureVerdictJson,
    pub reprocessing_refused: ExplainPostureVerdictJson,
}

/// One entry of a model's declared-fact probe set
/// (`docs/specs/cli.md` §"`smelt explain --json` output schema"):
/// `{"fact": "...", "probe": "<DiagnosticCode>", "cell": "...", "cadence":
/// "per_run"|"periodic"|"off", "cost": "<one line>"}`.
#[derive(Debug, Serialize)]
pub struct ExplainProbeJson {
    pub fact: String,
    pub probe: String,
    pub cell: String,
    pub cadence: String,
    pub cost: String,
}

/// Build the `--json --show-sql` report from the derived plan cells and
/// their built statement groups, plus the model's own Relation Contract
/// fill, its inbound edges' contracts, and the shared diagnostics builder's
/// per-cell technique-preview set and property set.
///
/// `diagnostics_cells` and `cell_verdicts` must both be in the same cell
/// order as `plan_cells`/`statements` —
/// `smelt_runtime::diagnostics::build_model_diagnostics` maps 1:1 over the
/// same `plan_cells` slice this report's own `statements` are built from,
/// so a positional `zip` is exact, not an approximation. `cell_verdicts` is
/// the model's property profile's own per-cell verdict
/// (`docs/specs/property_diff.md` §"The property profile") — this function
/// reads its `trigger`/`corner`/`technique`/`row_identity`/`contract_point`
/// fields verbatim rather than re-deriving them, so the report and the
/// profile structurally cannot disagree (`docs/specs/property_diff.md`
/// §Constraints item 4, "Report/profile parity").
#[allow(clippy::too_many_arguments)]
pub fn build_maintenance_plan_json(
    model_name: &str,
    plan_cells: &[PlanCell],
    statements: &[CellStatements],
    own_contract: RelationContractView,
    inbound_edges: Vec<InboundEdgeContract>,
    diagnostics_cells: &[PlanCellDiagnostics],
    cell_verdicts: &[smelt_logical::analysis::profile::CellVerdict],
    properties: PropertySet,
    state_columns: Vec<smelt_logical::StateColumnSummary>,
    execution_postures: Option<smelt_logical::ExecutionPostures>,
    is_snapshot_reconcile: Option<bool>,
    probe_entries: Vec<smelt_runtime::probe_plan::ProbePlanEntry>,
    cadence: smelt_core::config::ProbeCadence,
    refusals: Vec<smelt_logical::analysis::profile::ProfileRefusal>,
    pending_definition_delta: Option<&(MigrationVerdict, String)>,
    own_output_delta: Option<&smelt_logical::analysis::output_delta::OutputDelta>,
    key_locality: Option<&smelt_logical::maintenance::KeyLocality>,
) -> ExplainMaintenanceJson {
    let delta_signature = delta_signature_headline(own_output_delta, key_locality, &own_contract);
    let cadence_label = format_probe_cadence(cadence);
    let probes = probe_entries
        .into_iter()
        .map(|p| ExplainProbeJson {
            fact: p.fact,
            probe: p.probe,
            cell: p.cell,
            cadence: cadence_label.clone(),
            cost: p.cost,
        })
        .collect();
    let cells = plan_cells
        .iter()
        .zip(statements.iter())
        .zip(diagnostics_cells.iter())
        .zip(cell_verdicts.iter())
        .map(|(((cell, cs), diag_cell), verdict)| {
            let (no_statements_reason, statements) = match &cs.outcome {
                Ok(group) => {
                    let stmts = group
                        .statements
                        .iter()
                        .map(|s| ExplainStatementJson {
                            sql: s.sql.clone(),
                            // Every emitter today returns exactly one
                            // StatementGroup per cell, so every statement in
                            // it shares transactional_group == cell_index;
                            // statements sharing one index must run in the
                            // same transaction (`group.transactional`).
                            transactional_group: cs.cell_index,
                        })
                        .collect();
                    (None, stmts)
                }
                Err(reason) => (Some(reason.clone()), Vec::new()),
            };
            ExplainCellJson {
                group: verdict.group.clone(),
                trigger: verdict.trigger.clone(),
                corner: verdict.corner.clone(),
                technique: format!("{:?}", verdict.technique),
                row_identity: format!("{:?}", verdict.row_identity.identity),
                row_identity_proven_mismatch: verdict.row_identity.proven_mismatch.clone(),
                no_statements_reason,
                statements,
                technique_previews: diag_cell.technique_previews.clone(),
                contract_point: verdict.contract_point.clone(),
                state_downgrade: cell
                    .state_downgrade
                    .as_ref()
                    .map(|d| ExplainStateDowngradeJson {
                        original: format!("{:?}", d.original),
                        missing: d.missing.as_str().to_string(),
                        reason: d.reason.clone(),
                    }),
            }
        })
        .collect();
    let definition_delta =
        pending_definition_delta.map(|(verdict, plan_hash)| ExplainDefinitionDeltaJson {
            status: "pending".to_string(),
            verdict: format!("{:?}", verdict),
            plan_hash: plan_hash.clone(),
        });
    let execution_postures = execution_postures.map(|postures| {
        let run_shape = match is_snapshot_reconcile {
            Some(true) => "snapshot-reconcile",
            Some(false) => "window-forward",
            None => "n/a",
        };
        ExplainExecutionPosturesJson {
            run_shape: run_shape.to_string(),
            rerun_tolerant: ExplainPostureVerdictJson {
                holds: postures.rerun_tolerant.holds,
                reason: postures.rerun_tolerant.reason,
            },
            order_independent: ExplainPostureVerdictJson {
                holds: postures.order_independent.holds,
                reason: postures.order_independent.reason,
            },
            reprocessing_refused: ExplainPostureVerdictJson {
                holds: postures.reprocessing_refused.holds,
                reason: postures.reprocessing_refused.reason,
            },
        }
    });
    ExplainMaintenanceJson {
        model: model_name.to_string(),
        delta_signature,
        contract: own_contract,
        inbound_edges,
        cells,
        execution_postures,
        properties,
        state_columns,
        probes,
        refusals,
        definition_delta,
    }
}

/// Build the explain output from the dependency graph and config.
///
/// `origins` maps emitted model names to `(generator_file, generator_def_name)`.
/// `period` is the parsed `--period <start>..<end>` run window (bounds in
/// the model's own axis domain), when one was supplied — threaded into each
/// incremental model's `source_bounds` to resolve `scan_start`/`scan_end`
/// (`docs/specs/incremental_shapes.md` §"Observing the per-source clamp").
pub fn build_explain_output(
    graph: &DependencyGraph,
    config: &Config,
    fn_bodies: &smelt_runtime::FnBodyMap,
    origins: &std::collections::HashMap<String, (String, String)>,
    period: Option<&(String, String)>,
) -> Result<ExplainOutput> {
    let execution_order = graph.execution_order()?;

    let mut models = BTreeMap::new();
    for model_name in &execution_order {
        let model_file = graph.get_model(model_name)?;
        let metadata = model_file.metadata.as_deref();
        let frontmatter = smelt_planner::Frontmatter::parse(&model_file.content);

        let materialization = config.get_materialization_with_metadata(model_name, metadata);
        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
        let tags = config.get_tags(model_name, metadata);

        let incremental = match (inc_config, ts_config) {
            (Some(inc), Some(ts)) => {
                // Classify on the *expanded* SQL so a RANGE BETWEEN INTERVAL
                // declared inside a `smelt.define` body is seen.
                let expanded_sql =
                    smelt_runtime::expand_function_calls(&model_file.content, fn_bodies);
                let batch_safety =
                    compute_batch_safety_label(model_name, &expanded_sql, model_file, &inc, &ts);
                let source_bounds =
                    compute_source_bounds(model_name, &expanded_sql, graph, config, period);
                Some(ExplainIncremental {
                    granularity: ts.granularity,
                    partition_column: ts.partition_column.clone(),
                    event_time_column: ts.event_time_column.clone(),
                    unique_key: inc.unique_key.clone(),
                    batch_safety,
                    source_bounds,
                })
            }
            _ => None,
        };

        let owner = metadata.and_then(|m| m.owner.clone());
        let dependencies = graph.get_upstream(model_name);

        // Build origin for generator-emitted models.
        let origin = origins
            .get(model_name)
            .map(|(gf, gn)| ModelOriginKind::Generated {
                generator_file: gf.clone(),
                generator_name: gn.clone(),
            });

        // Emit `refresh: "incremental"` when the model is keyed
        // (`refresh: incremental` + `grain: key`); omit otherwise.
        let refresh = metadata
            .filter(|m| m.is_keyed())
            .and_then(|m| m.refresh.clone());

        models.insert(
            model_name.clone(),
            ExplainModel {
                dependencies,
                materialization,
                refresh,
                incremental,
                tags,
                owner,
                origin,
            },
        );
    }

    Ok(ExplainOutput {
        models,
        execution_order,
        physical: None,
    })
}

/// Build the physical explain section from the plan summary and graph.
///
/// The physical section lists per-model strategy (from the `PlanSummary`),
/// ephemerals (from the graph), and planner transformations.
pub fn build_physical_explain(
    plan_summary: &smelt_runtime::PlanSummary,
    graph: &DependencyGraph,
    config: &Config,
    target: &str,
) -> ExplainPhysical {
    let mut nodes = BTreeMap::new();
    let mut ephemerals = Vec::new();

    for record in &plan_summary.models {
        let model_name = &record.name;

        // Collect ephemerals
        if matches!(record.strategy, smelt_runtime::ModelStrategy::Ephemeral) {
            ephemerals.push(model_name.clone());
            continue;
        }

        let strategy = match &record.strategy {
            smelt_runtime::ModelStrategy::FullRefresh => "full_refresh".to_string(),
            smelt_runtime::ModelStrategy::Incremental {
                partition_column,
                granularity,
            } => format!(
                "incremental (partition: {}, granularity: {})",
                partition_column, granularity
            ),
            smelt_runtime::ModelStrategy::Keyed => "cumulative_aggregate".to_string(),
            smelt_runtime::ModelStrategy::MaterializedView => "materialized_view".to_string(),
            smelt_runtime::ModelStrategy::Ephemeral => "ephemeral".to_string(),
            smelt_runtime::ModelStrategy::Skipped { reason } => {
                format!("skipped ({})", reason)
            }
        };

        let model_target = graph
            .get_model(model_name)
            .ok()
            .map(|m| config.get_target(model_name, m.metadata.as_deref(), target))
            .unwrap_or_else(|| target.to_string());

        nodes.insert(
            model_name.clone(),
            ExplainPhysicalNode {
                strategy,
                materialization: record.materialization.clone(),
                target: model_target,
                logical_origins: vec![model_name.clone()],
            },
        );
    }

    // Any ephemeral-only models from the graph that aren't in the PlanSummary
    // (e.g., if PlanSummary omitted them) — scan the graph for completeness.
    for (model_name, _) in graph.iter_models() {
        let mat = graph
            .get_model(model_name)
            .ok()
            .map(|m| config.get_materialization_with_metadata(model_name, m.metadata.as_deref()))
            .unwrap_or(Materialization::View);
        if mat == Materialization::Ephemeral && !ephemerals.contains(&model_name.to_string()) {
            ephemerals.push(model_name.to_string());
        }
    }

    let execution_order: Vec<String> = plan_summary
        .models
        .iter()
        .filter(|r| !matches!(r.strategy, smelt_runtime::ModelStrategy::Ephemeral))
        .map(|r| r.name.clone())
        .collect();

    ExplainPhysical {
        execution_order,
        nodes,
        ephemerals,
        transformations: vec![],
    }
}

/// Derive per-source bounds for a model, resolving each `Bounded` source's
/// run-relative scan window against `period` when one is supplied — via the
/// same `smelt_logical::resolve_scan_window` a run's pushdown filter uses
/// (`docs/specs/incremental_shapes.md` §"Observing the per-source clamp").
fn compute_source_bounds(
    model_name: &str,
    sql: &str,
    graph: &DependencyGraph,
    config: &Config,
    period: Option<&(String, String)>,
) -> BTreeMap<String, SourceBoundJson> {
    use smelt_planner::analysis::source_bounds::derive_model_bounds;
    use smelt_planner::Frontmatter;

    let ctx = build_bound_context(model_name, graph, config);

    let stripped = Frontmatter::strip(sql);
    let raw_bounds = derive_model_bounds(stripped, &ctx);

    let mut result = BTreeMap::new();
    for (source_name, bound) in raw_bounds {
        let json = match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                let (scan_start, scan_end, scan_unresolved) = match period {
                    Some((run_start, run_end)) => {
                        let axis =
                            smelt_runtime::windowing::axis_implied_by_literal_form(Some(run_start));
                        match smelt_logical::resolve_scan_window(
                            axis,
                            run_start,
                            run_end,
                            &smelt_logical::Offset::Seconds(before),
                            &smelt_logical::Offset::Seconds(after),
                        ) {
                            smelt_logical::ScanWindowVerdict::Resolved { start, end } => {
                                (Some(start), Some(end), None)
                            }
                            smelt_logical::ScanWindowVerdict::Unresolved { reason } => {
                                (None, None, Some(reason))
                            }
                        }
                    }
                    None => (None, None, None),
                };
                SourceBoundJson::Bounded {
                    partition_col: source_partition_col,
                    before: before.to_iso8601(),
                    after: after.to_iso8601(),
                    scan_start,
                    scan_end,
                    scan_unresolved,
                }
            }
            BoundResult::Unbounded => SourceBoundJson::Unbounded,
            BoundResult::NotDerivable => SourceBoundJson::NotDerivable,
        };
        result.insert(source_name, json);
    }
    result
}

fn compute_batch_safety_label(
    name: &str,
    sql: &str,
    model_file: &ModelFile,
    inc: &PartitionGrainConfig,
    ts: &TimeseriesConfig,
) -> String {
    let model_info = ModelInfo {
        name: name.to_string(),
        sql: sql.to_string(),
        refs: model_file
            .refs
            .iter()
            .map(|r| r.smelt_ref.to_path().join("."))
            .collect(),
        incremental_config: Some(inc.clone()),
        timeseries_config: Some(ts.clone()),
        plausible_columns: Default::default(),
    };
    match analyze_batch_safety(&model_info) {
        BatchSafety::FullyBatchSafe => "fully_batch_safe".to_string(),
        BatchSafety::BoundedSafe {
            max_chunk_days,
            context_days,
            ..
        } => format!(
            "bounded_safe(chunk={}d,context={}d)",
            max_chunk_days, context_days
        ),
        BatchSafety::PerPartitionOnly { .. } => "per_partition_only".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ModelConfig, Target};
    use crate::discovery::ModelKind;
    use rowan::TextRange;
    use smelt_core::RefInfo;
    use std::collections::HashMap;

    fn make_model(name: &str, deps: Vec<&str>, content: &str) -> crate::discovery::ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: smelt_core::refs::SmeltRef::Path(vec![dep.to_string()]),
            })
            .collect();

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        crate::discovery::ModelFile {
            name: name.to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: content.to_string(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: ModelKind::Sql,
            address_segments: vec![name.to_string()],
        }
    }

    fn make_config(model_configs: Vec<(&str, ModelConfig)>) -> Config {
        let mut models = HashMap::new();
        for (name, mc) in model_configs {
            models.insert(name.to_string(), mc);
        }

        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
                warehouse: None,
                format: None,
                settings: None,
                project: None,
                dataset: None,
                location: None,
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models,
            python: None,
            target: None,
            state: Default::default(),
            maintenance: None,
            probes: Default::default(),
        }
    }

    #[test]
    fn test_batch_safety_uses_expanded_function_body() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::Granularity;

        // A model whose only lookback lives inside a `smelt.define` body must
        // classify as `bounded_safe` — but only when the explain path expands
        // the function. With no registry the outer SQL shows no lookback and it
        // falls back to `fully_batch_safe`. This is the classification-path
        // counterpart to the execution-path expansion.
        let content = "SELECT device_id, d FROM smelt.functions.windowed(src => raw_events)";
        let models = vec![make_model("sessions", vec![], content)];
        let config = make_config(vec![(
            "sessions",
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: Some(TimeseriesConfig {
                    event_time_column: "d".to_string(),
                    partition_column: "d".to_string(),
                    granularity: Granularity::Day,
                    week_start: None,
                    assert_monotonic: false,
                }),
                refresh: Some(smelt_core::config::RefreshStrategy::Incremental),
                grain: Some(smelt_core::config::Grain::Partition),
                unique_key: None,
                safety_overrides: None,
                batched_retired: (),
                merge_key: None,
                tags: vec![],
                target: None,
                format: None,
            },
        )]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let mut fn_bodies: smelt_runtime::FnBodyMap = HashMap::new();
        fn_bodies.insert(
            "windowed".to_string(),
            (
                vec![("src".to_string(), None)],
                "(SELECT device_id, d, LAG(d) OVER (PARTITION BY device_id ORDER BY d \
                 RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS p FROM src)"
                    .to_string(),
            ),
        );

        let bs = |fns: &smelt_runtime::FnBodyMap| {
            build_explain_output(&graph, &config, fns, &HashMap::new(), None)
                .unwrap()
                .models["sessions"]
                .incremental
                .as_ref()
                .unwrap()
                .batch_safety
                .clone()
        };

        let with_registry = bs(&fn_bodies);
        let without_registry = bs(&HashMap::new());

        assert!(
            with_registry.starts_with("bounded_safe"),
            "with the registry the function-internal RANGE is seen: {with_registry}"
        );
        assert_eq!(
            without_registry, "fully_batch_safe",
            "without the registry the outer SQL shows no lookback: {without_registry}"
        );
    }

    #[test]
    fn test_explain_basic() {
        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.orders GROUP BY date",
            ),
        ];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();

        assert_eq!(output.execution_order.len(), 2);
        assert_eq!(output.execution_order[0], "orders");
        assert_eq!(output.execution_order[1], "daily_revenue");
        assert_eq!(output.models.len(), 2);

        let orders = &output.models["orders"];
        assert!(orders.dependencies.is_empty());
        assert_eq!(orders.materialization, Materialization::View);
        assert!(orders.incremental.is_none());

        let daily = &output.models["daily_revenue"];
        assert_eq!(daily.dependencies, vec!["orders"]);
    }

    #[test]
    fn test_explain_with_incremental() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::Granularity;

        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.orders GROUP BY date",
            ),
        ];
        let config = make_config(vec![(
            "daily_revenue",
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: Some(TimeseriesConfig {
                    event_time_column: "created_at".to_string(),
                    partition_column: "order_date".to_string(),
                    granularity: Granularity::Day,
                    week_start: None,
                    assert_monotonic: false,
                }),
                refresh: Some(smelt_core::config::RefreshStrategy::Incremental),
                grain: Some(smelt_core::config::Grain::Partition),
                unique_key: None,
                safety_overrides: None,
                batched_retired: (),
                merge_key: None,
                tags: vec!["revenue".to_string(), "daily".to_string()],
                target: None,
                format: None,
            },
        )]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();

        let daily = &output.models["daily_revenue"];
        assert_eq!(daily.materialization, Materialization::Table);
        assert_eq!(daily.tags, vec!["revenue", "daily"]);

        let inc = daily.incremental.as_ref().unwrap();
        assert_eq!(inc.partition_column, "order_date");
        assert_eq!(inc.event_time_column, "created_at");
        assert_eq!(inc.batch_safety, "fully_batch_safe");
    }

    #[test]
    fn test_explain_json_serialization() {
        let models = vec![make_model("a", vec![], "SELECT 1")];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();
        let json = serde_json::to_string_pretty(&output).unwrap();

        assert!(json.contains("\"models\""));
        assert!(json.contains("\"execution_order\""));
        assert!(json.contains("\"a\""));
    }

    #[test]
    fn test_explain_with_owner_from_metadata() {
        use crate::metadata::ModelMetadata;

        let mut model = make_model("orders", vec![], "SELECT 1");
        model.metadata = Some(Box::new(ModelMetadata {
            owner: Some("analytics-team".to_string()),
            ..Default::default()
        }));

        let models = vec![model];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();
        assert_eq!(
            output.models["orders"].owner.as_deref(),
            Some("analytics-team")
        );
    }

    /// `smelt explain --json` must emit `"materialization": "table"` and
    /// `"refresh": "incremental"` for a keyed model (`refresh: incremental`
    /// and `grain: key`), and must NOT emit `"cumulative_aggregate"` anywhere
    /// in the materialization field.
    ///
    /// Spec oracle: `docs/specs/cli.md` §"`smelt explain --json` output schema".
    #[test]
    fn explain_json_emits_refresh_keyed_for_keyed_model() {
        use crate::metadata::ModelMetadata;
        use smelt_core::config::RefreshStrategy;

        let mut model = make_model(
            "device_stats",
            vec![],
            "SELECT device_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id",
        );
        model.metadata = Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(smelt_core::config::Grain::Key),
            ..Default::default()
        }));

        let models = vec![model];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();

        let model_entry = &output.models["device_stats"];

        // The `materialization` field must be `table` (the storage kind).
        assert_eq!(
            model_entry.materialization,
            Materialization::Table,
            "keyed model materialization must be 'table', not anything else"
        );

        // The `refresh` field must be `Some(Incremental)`.
        assert_eq!(
            model_entry.refresh,
            Some(RefreshStrategy::Incremental),
            "keyed model must have refresh: Some(Incremental)"
        );

        // Verify the JSON serialization: must emit `"refresh": "incremental"`
        // and `"materialization": "table"`, must NOT contain `"cumulative_aggregate"`.
        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(
            json.contains("\"refresh\": \"incremental\""),
            "JSON must contain '\"refresh\": \"incremental\"'; got:\n{json}"
        );
        assert!(
            json.contains("\"materialization\": \"table\""),
            "JSON must contain '\"materialization\": \"table\"'; got:\n{json}"
        );
        assert!(
            !json.contains("\"cumulative_aggregate\""),
            "JSON must not contain '\"cumulative_aggregate\"' in the materialization field; got:\n{json}"
        );
    }

    /// A plain `materialization: table` model (no `refresh: keyed`) must
    /// NOT emit a `refresh` field in the JSON — the field is omitted for
    /// the default full-refresh strategy.
    #[test]
    fn explain_json_omits_refresh_for_full_refresh_model() {
        let mut model = make_model("orders", vec![], "SELECT * FROM raw_orders");
        model.metadata = Some(Box::new(crate::metadata::ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: None,
            ..Default::default()
        }));

        let models = vec![model];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new(), None).unwrap();

        let model_entry = &output.models["orders"];
        assert_eq!(
            model_entry.refresh, None,
            "full-refresh model must have no refresh field"
        );

        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(
            !json.contains("\"refresh\""),
            "full-refresh model JSON must not emit a 'refresh' field; got:\n{json}"
        );
    }

    // The `Technique::DeleteInsert`-over-a-`TableExpr`-FROM clamp-ordering
    // regression this module used to cover directly (via its own
    // `build_cell_statement_group`) now lives with the logic that owns it:
    // `smelt_runtime::diagnostics::build_technique_statements` for the
    // symbolic no-`--period` case
    // (`crates/smelt-runtime/src/diagnostics.rs::tests::
    // delete_insert_clamp_succeeds_on_table_expr_function_from`), and
    // `crates/smelt-cli/tests/explain_model.rs::sessions_show_sql_emits_statements`
    // for the real-`--period` case this file's own
    // `build_delete_insert_period_statement_group` still derives.
}
