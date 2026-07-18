use crate::discovery::ModelFile;
use anyhow::Result;
use serde::Serialize;
use smelt_core::config::{Config, Grain, RefreshStrategy, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::{BatchedConfig, Granularity, Materialization, ModelOriginKind};
use smelt_logical::maintenance::emit::{MaintenanceDialect, StatementGroup};
use smelt_logical::maintenance::{PlanCell, Technique};
use smelt_planner::{analyze_batch_safety, BatchSafety, BoundContext, BoundResult, ModelInfo};
use smelt_runtime::{
    classify_cumulative_sql, inject_source_filters, inject_time_filter, CompilerRegistry,
    EphemeralResolver, SourceBound, TimeRange,
};
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

/// The clock slot's shared fields (`docs/specs/models.md` §"The Relation
/// Contract": clock and identity "carry identical field paths" across
/// both providers). Rendered identically whichever provider filled it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractClock {
    pub event_time_column: String,
    pub partition_column: String,
    pub granularity: Granularity,
}

/// One provider's fill of the Relation Contract's **declared-and-checked**
/// shape-defining slots — the clock and identity — plus the derived
/// `grain` label that summarizes them (`docs/specs/models.md` §"Refresh
/// axis", §"The Relation Contract"; `docs/specs/sources.md` §"The source
/// as a Relation Contract provider"). Both a source and a model output are
/// rendered through this one struct — a consumer never needs to know
/// which provider filled it (`clock`/`identity` are the two fields every
/// provider fills through the same field paths).
///
/// `clock`/`identity`/`derived_grain` are all `None` for a provider that
/// declares neither fact — legal for a source (no admission gate to
/// fail), reported rather than refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<RelationContractClock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_grain: Option<Grain>,
}

impl RelationContractView {
    /// Provider-agnostic construction from the raw declared facts — reads
    /// `Option<&TimeseriesConfig>` / `Option<&[String]>` directly rather
    /// than a `SourceInfo` or `ModelMetadata`, so there is exactly one
    /// derivation reused by both providers
    /// (`smelt_core::config::derive_grain`), never a source-specific or
    /// model-specific reimplementation.
    pub fn from_facts(
        timeseries: Option<&TimeseriesConfig>,
        unique_key: Option<&[String]>,
    ) -> Self {
        let derived_grain = smelt_core::config::derive_grain(
            timeseries.is_some(),
            unique_key,
            timeseries.map(|t| t.partition_column.as_str()),
        );
        RelationContractView {
            clock: timeseries.map(|ts| RelationContractClock {
                event_time_column: ts.event_time_column.clone(),
                partition_column: ts.partition_column.clone(),
                granularity: ts.granularity,
            }),
            identity: unique_key.map(|k| k.to_vec()),
            derived_grain,
        }
    }
}

/// Which provider filled one inbound edge's [`RelationContractView`] — a
/// declared `sources.*` ref or an upstream maintained model
/// (`docs/specs/incremental_models.md` §"Upstream model edges": the graph
/// layer treats both edge kinds as the same standing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationContractProvider {
    Source,
    Model,
}

/// One inbound edge's provider identity and Relation Contract fill
/// (`docs/specs/models.md` §"The Relation Contract").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InboundEdgeContract {
    pub name: String,
    pub provider: RelationContractProvider,
    pub contract: RelationContractView,
}

impl InboundEdgeContract {
    pub fn source(name: String, contract: RelationContractView) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Source,
            contract,
        }
    }

    pub fn model(name: String, contract: RelationContractView) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Model,
            contract,
        }
    }
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

/// Assemble a model's own [`RelationContractView`] plus its inbound edges'
/// contracts (`docs/specs/models.md` §"The Relation Contract").
///
/// Model-to-model edges come from `model_upstream`
/// (`DependencyGraph::get_upstream`) — resolved against `models` for each
/// upstream's own declared clock/identity. Source edges are read directly
/// off `model`'s own `smelt.sources.*` refs (`model.refs`), because the
/// graph layer's dependency map excludes per-entity source refs entirely
/// (`smelt_core::graph::DependencyGraph::build` filters `first == "sources"`
/// out of `deps`) — the graph layer's model/source distinction, not a
/// second one invented here. Edges are sorted by name for a deterministic
/// report.
pub fn build_relation_contract(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[smelt_core::SourceInfo],
) -> (RelationContractView, Vec<InboundEdgeContract>) {
    let own_metadata = model.metadata.as_deref();
    let own_contract = RelationContractView::from_facts(
        own_metadata.and_then(|m| m.timeseries.as_ref()),
        own_metadata.and_then(|m| m.unique_key.as_deref()),
    );

    let mut edges: Vec<InboundEdgeContract> = model_upstream
        .iter()
        .filter_map(|name| {
            models
                .iter()
                .find(|m| &m.canonical_path() == name)
                .map(|m| {
                    let md = m.metadata.as_deref();
                    InboundEdgeContract::model(
                        name.clone(),
                        RelationContractView::from_facts(
                            md.and_then(|m| m.timeseries.as_ref()),
                            md.and_then(|m| m.unique_key.as_deref()),
                        ),
                    )
                })
        })
        .collect();

    for r in &model.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(String::as_str) != Some("sources") {
            continue;
        }
        // `SourceInfo::address_segments` is the full scan-root-stripped path,
        // `sources` segment included (`discover_source_infos` /
        // `ModelDiscovery::compute_address_segments`) — the same segments a
        // `smelt.sources.<...>` ref carries, so no prefix-stripping here.
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        let name = segs.join(".");
        if edges.iter().any(|e| e.name == name) {
            continue;
        }
        edges.push(InboundEdgeContract::source(
            name,
            RelationContractView::from_facts(info.timeseries.as_ref(), info.unique_key.as_deref()),
        ));
    }

    edges.sort_by(|a, b| a.name.cmp(&b.name));

    (own_contract, edges)
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
/// when the model declares none) — read only to look up each cell's active
/// `write:` pin, if any (`docs/specs/incremental_models.md` §"Per-cell
/// write addressing").
pub fn build_maintenance_plan_report(
    model_name: &str,
    result: &smelt_db::queries::maintenance::MaintenancePlanResult,
    own_contract: &RelationContractView,
    edges: &[InboundEdgeContract],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
) -> String {
    use smelt_logical::maintenance::PartitionLocal;
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "Maintenance plan: {}", model_name);
    let _ = writeln!(out);

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
            let _ = writeln!(out, "      ledger_catch_up: {}", cell.ledger_catch_up);
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
            match smelt_db::queries::maintenance::matching_write_pin(
                cell,
                &result.column_groups,
                cells_cfg,
            ) {
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
        }
    }
    let _ = writeln!(out);

    // Key temporal locality (`incremental_models.md` §"Key temporal
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
        // (`incremental_models.md` §"What the composed shape uniquely
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
        }
    }

    out
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

impl RegionLiterals {
    /// The raw (unquoted) start/end bounds. Callers quote them as needed
    /// per the destination emitter's own literal convention (a `Region`'s
    /// `start`/`end` are pre-quoted SQL literals; `TimeRange`'s are quoted
    /// by `inject_time_filter` itself).
    fn raw(&self) -> (String, String) {
        match self {
            RegionLiterals::Period { start, end } => (start.clone(), end.clone()),
            RegionLiterals::Placeholders => {
                ("{{window_start}}".to_string(), "{{window_end}}".to_string())
            }
        }
    }

    fn quoted_region(&self) -> smelt_logical::maintenance::emit::Region {
        let (start, end) = self.raw();
        smelt_logical::maintenance::emit::Region {
            start: format!("'{}'", start.replace('\'', "''")),
            end: format!("'{}'", end.replace('\'', "''")),
        }
    }
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

/// Build the [`StatementGroup`] a plan cell would execute, using the same
/// pure emitters a run executes
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)"):
/// this is the CLI-side "same inputs, same emitter" observation path —
/// `--show-sql` never connects to a backend or executes anything, so every
/// SELECT body here is compiled through the sanctioned
/// `CompilerRegistry::get(...).compile_with_sql_and_ephemerals(...)` entry
/// point (Run-pipeline-parity rule, `architecture.md`) and every clamp is
/// injected via `smelt-runtime`'s existing transformer functions, never a
/// new compile helper in `smelt-cli`.
///
/// Returns `Err(reason)` rather than fabricating SQL when this cell's
/// technique-specific inputs (unique key, keyed-fold driving-source
/// classification) cannot be assembled from the discovered project, or when
/// the technique has no production consumer yet (`Technique::InPlaceUpdate`,
/// `incremental_models.md` — no live plan cell lowers to it).
///
/// `resolver` must be built from the *actual* discovered project (the same
/// way `smelt-runtime`'s dry-run compile path in `execute.rs` builds it —
/// `SqlCompiler::build_ephemeral_resolver` over the target's ephemeral
/// models), not `EphemeralResolver::empty()`: an empty resolver leaves any
/// `smelt.<ephemeral>` ref in this model's SELECT body resolved as a
/// physical table reference instead of CTE-inlined, which is what a real
/// run would do (`docs/specs/cli.md` §"`smelt explain <model>`
/// maintenance-plan report").
#[allow(clippy::too_many_arguments)]
pub fn build_cell_statement_group(
    cell: &PlanCell,
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    unique_key: &[String],
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    region: &RegionLiterals,
    derived: Option<&DerivedWindow>,
) -> Result<StatementGroup, String> {
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let table_name = format!("{schema}.{}", model.db_name_owned());

    match cell.technique {
        Technique::DeleteInsert => {
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

            // Inject the output clamp on the model's own (uncompiled) SQL
            // *before* compiling — the same order the live run uses
            // (`derive_batch_filtered_sql` clamps `clean_sql` — the raw model
            // body — then compiles the clamped text;
            // `docs/specs/model_transforms.md` §Semantics "Source-filter
            // pushdown + the two clamps"). `inject_time_filter` only needs
            // the model's own outermost FROM clause, which is a plain
            // CTE/table reference regardless of what a nested CTE's FROM
            // contains, so clamping the raw body is always safe. Clamping
            // the *compiled* SQL instead (the previous order here) fed
            // `inject_time_filter`'s reparse a FROM position that the
            // compiler's printer had already expanded — for a model whose
            // FROM is a `TableExpr`-returning function call
            // (`smelt.functions.f(...)`), that expansion emits a
            // parenthesized form that smelt-parser's grammar cannot
            // re-parse (it round-trips fine as DuckDB-executable text, which
            // is all the live run ever does with it — it never feeds
            // compiled SQL back through the parser), so the reparse-based
            // `inject_time_filter` call spuriously failed with "No FROM
            // clause found". Wrapping before compiling never asks the parser
            // to re-parse the compiler's own function-expansion output, so
            // this is a call-ordering fix, not a new clamp mechanism.
            let (compiled, region_used) = if let Some(dw) = derived {
                // A concrete `--period` was given: derive the statements
                // exactly as a live run's `derive_batch_filtered_sql` would
                // for this window — output clamp *and* the per-source
                // widened-scan pushdown, composed in one single-owner call
                // rather than re-implemented here.
                let run_range = TimeRange {
                    start: dw.output_start.clone(),
                    end: dw.output_end.clone(),
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

                let region_used = smelt_logical::maintenance::emit::Region {
                    start: format!("'{}'", dw.output_start.replace('\'', "''")),
                    end: format!("'{}'", dw.output_end.replace('\'', "''")),
                };
                (compiled, region_used)
            } else {
                // No `--period`: symbolic placeholders, no window to derive
                // a skew inversion or scan margin from — output-clamp the
                // raw body with the placeholder literals only.
                let (raw_start, raw_end) = region.raw();
                let wrapped_raw = inject_time_filter(
                    &stripped_sql,
                    &partition_col,
                    &TimeRange {
                        start: raw_start,
                        end: raw_end,
                    },
                )
                .map_err(|e| format!("failed to inject the output clamp: {e}"))?;

                let compiled = registry
                    .get(target)
                    .compile_with_sql_and_ephemerals(model, schema, &wrapped_raw, resolver)
                    .map_err(|e| format!("failed to compile model body: {e}"))?;

                (compiled, region.quoted_region())
            };

            Ok(smelt_logical::maintenance::emit::emit_delete_insert(
                &table_name,
                &partition_col,
                &region_used,
                &compiled.sql,
                dialect,
            ))
        }
        Technique::KeyedFold => {
            let model_has_timeseries = model
                .metadata
                .as_ref()
                .is_some_and(|m| m.timeseries.is_some());
            let classification = classify_cumulative_sql(
                &model.name,
                &stripped_sql,
                source_timeseries,
                model_has_timeseries,
            )
            .map_err(|e| format!("{e}"))?;

            let (raw_start, raw_end) = region.raw();
            let time_range = TimeRange {
                start: raw_start,
                end: raw_end,
            };
            let mut bound_map = std::collections::HashMap::new();
            bound_map.insert(
                classification.driving_source.name.clone(),
                SourceBound {
                    partition_col: classification
                        .driving_source
                        .timeseries
                        .partition_column
                        .clone(),
                    before_secs: 0,
                    after_secs: 0,
                },
            );
            let pushed = inject_source_filters(&stripped_sql, &bound_map, &time_range);

            let compiled = registry
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &pushed, resolver)
                .map_err(|e| format!("failed to compile model body: {e}"))?;

            let folds: Vec<(String, String)> = classification
                .aggregator_columns
                .iter()
                .map(|col| {
                    let target_col = format!("target.{}", col.output_name);
                    let delta_col = format!("delta.{}", col.output_name);
                    (
                        col.output_name.clone(),
                        col.cross_partition_combiner.render(&target_col, &delta_col),
                    )
                })
                .collect();

            Ok(smelt_logical::maintenance::emit::emit_keyed_fold(
                &table_name,
                &classification.unique_key,
                &folds,
                &compiled.sql,
                // `smelt explain` does not yet render the locality-derived
                // target-scan slice predicate — that surface lands in
                // `docs/plans/20260715-composed-axes-conditional-
                // maintenance.md` phase A5.
                None,
                dialect,
            ))
        }
        Technique::ColumnScopedMerge => {
            if unique_key.is_empty() {
                return Err(
                    "no unique_key declared — cannot build the column-scoped MERGE".to_string(),
                );
            }
            let compiled = registry
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &stripped_sql, resolver)
                .map_err(|e| format!("failed to compile model body: {e}"))?;

            Ok(smelt_logical::maintenance::emit::emit_column_scoped_merge(
                &table_name,
                unique_key,
                &compiled.sql,
                dialect,
            ))
        }
        Technique::InPlaceUpdate => Err("Technique::InPlaceUpdate has no production consumer yet \
             (docs/specs/incremental_models.md § Known Divergences)"
            .to_string()),
    }
}

/// Build a [`CellStatements`] entry for every cell in `plan`, in the same
/// order they appear in the report (`docs/specs/cli.md`: "Statements print
/// in execution order").
#[allow(clippy::too_many_arguments)]
pub fn build_all_cell_statements(
    plan_cells: &[PlanCell],
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    unique_key: &[String],
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    region: &RegionLiterals,
    derived: Option<&DerivedWindow>,
) -> Vec<CellStatements> {
    plan_cells
        .iter()
        .enumerate()
        .map(|(cell_index, cell)| CellStatements {
            cell_index,
            outcome: build_cell_statement_group(
                cell,
                model,
                schema,
                target,
                registry,
                resolver,
                dialect,
                unique_key,
                source_timeseries,
                region,
                derived,
            ),
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
/// `smelt explain <model> --show-sql` and `smelt run`/`smelt backbuild
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
}

/// The `--json --show-sql` per-model report:
/// `{"model": "<name>", "contract": {...}, "inbound_edges": [...], "cells":
/// [...]}`, carrying the Relation Contract slots
/// (`docs/specs/models.md` §"The Relation Contract") for both this model
/// and every inbound edge alongside each cell's own `statements` array
/// (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
/// report").
#[derive(Debug, Serialize)]
pub struct ExplainMaintenanceJson {
    pub model: String,
    pub contract: RelationContractView,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inbound_edges: Vec<InboundEdgeContract>,
    pub cells: Vec<ExplainCellJson>,
}

/// Build the `--json --show-sql` report from the derived plan cells and
/// their built statement groups, plus the model's own Relation Contract
/// fill and its inbound edges' contracts.
pub fn build_maintenance_plan_json(
    model_name: &str,
    plan_cells: &[PlanCell],
    statements: &[CellStatements],
    own_contract: RelationContractView,
    inbound_edges: Vec<InboundEdgeContract>,
) -> ExplainMaintenanceJson {
    let cells = plan_cells
        .iter()
        .zip(statements.iter())
        .map(|(cell, cs)| {
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
                group: cell.group.clone(),
                trigger: format!("{:?}", cell.trigger),
                corner: format!("{:?}", cell.corner),
                technique: format!("{:?}", cell.technique),
                row_identity: format!("{:?}", cell.row_identity.identity),
                row_identity_proven_mismatch: cell.row_identity.proven_mismatch.clone(),
                no_statements_reason,
                statements,
            }
        })
        .collect();
    ExplainMaintenanceJson {
        model: model_name.to_string(),
        contract: own_contract,
        inbound_edges,
        cells,
    }
}

/// Build the explain output from the dependency graph and config.
///
/// `origins` maps emitted model names to `(generator_file, generator_def_name)`.
pub fn build_explain_output(
    graph: &DependencyGraph,
    config: &Config,
    fn_bodies: &smelt_runtime::FnBodyMap,
    origins: &std::collections::HashMap<String, (String, String)>,
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
                let source_bounds = compute_source_bounds(model_name, &expanded_sql, graph, config);
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

/// Derive per-source bounds for a model.
fn compute_source_bounds(
    model_name: &str,
    sql: &str,
    graph: &DependencyGraph,
    config: &Config,
) -> BTreeMap<String, SourceBoundJson> {
    use smelt_planner::analysis::source_bounds::derive_model_bounds;
    use smelt_planner::Frontmatter;

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

    let stripped = Frontmatter::strip(sql);
    let raw_bounds = derive_model_bounds(stripped, &ctx);

    let mut result = BTreeMap::new();
    for (source_name, bound) in raw_bounds {
        let json = match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => SourceBoundJson::Bounded {
                partition_col: source_partition_col,
                before: before.to_iso8601(),
                after: after.to_iso8601(),
            },
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
    inc: &BatchedConfig,
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
        }
    }

    #[test]
    fn test_batch_safety_uses_expanded_function_body() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::{BatchedConfig, Granularity};

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
                batched: Some(BatchedConfig {
                    unique_key: vec![],
                    nondeterministic_columns: vec![],
                    safety_overrides: Default::default(),
                }),
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
            build_explain_output(&graph, &config, fns, &HashMap::new())
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
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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
        use smelt_core::{BatchedConfig, Granularity};

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
                batched: Some(BatchedConfig {
                    unique_key: vec![],
                    nondeterministic_columns: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec!["revenue".to_string(), "daily".to_string()],
                target: None,
                format: None,
            },
        )]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();
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
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();
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
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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

    /// `build_cell_statement_group`'s `Technique::DeleteInsert` branch must
    /// succeed for a model whose outermost FROM is a `TableExpr`-returning
    /// function call (`smelt.functions.windowed(...)`), the shape
    /// `silver.sessions` uses in `examples/web_analytics`
    /// (`crates/smelt-cli/tests/explain_model.rs::sessions_show_sql_emits_statements`
    /// is the real-fixture counterpart of this unit test). Clamping the raw,
    /// unexpanded model body *before* compiling — never the compiled output,
    /// which the compiler's own printer has already expanded — is what makes
    /// this succeed: `inject_time_filter`'s reparse only ever sees a plain
    /// FROM clause referencing a CTE, never the function-expansion's
    /// reparse-hostile output.
    #[test]
    fn delete_insert_clamp_succeeds_on_table_expr_function_from_after_expansion() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_logical::maintenance::{Corner, PartitionLocal, Trigger};

        let content = "SELECT device_id, d FROM smelt.functions.windowed(src => raw_events)";
        let mut model = make_model("sessions", vec!["raw_events"], content);
        model.metadata = Some(Box::new(crate::metadata::ModelMetadata {
            materialization: Some(Materialization::Table),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "d".to_string(),
                partition_column: "d".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        }));

        let config = make_config(vec![]);
        let mut registry = CompilerRegistry::new(&config, &config.targets);
        let mut fn_bodies: smelt_runtime::FnBodyMap = HashMap::new();
        fn_bodies.insert(
            "windowed".to_string(),
            (
                vec![("src".to_string(), None)],
                "(SELECT * FROM src)".to_string(),
            ),
        );
        registry.set_function_bodies_all(fn_bodies);
        let resolver = registry.get("dev").build_ephemeral_resolver(&[], "main");

        let cell = PlanCell {
            group: "{*}".to_string(),
            trigger: Trigger::Backfill,
            corner: Corner::RecomputeRegion,
            technique: Technique::DeleteInsert,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: smelt_logical::maintenance::RowIdentityVerdict {
                identity: smelt_logical::maintenance::RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
        };

        let source_timeseries = smelt_planner::SourceTimeseriesMap::new();
        let region = RegionLiterals::Placeholders;

        let outcome = build_cell_statement_group(
            &cell,
            &model,
            "main",
            "dev",
            &registry,
            &resolver,
            smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
            &[],
            &source_timeseries,
            &region,
            None,
        );

        let group = outcome.unwrap_or_else(|e| {
            panic!("expected Ok (clamp injection over the expanded FROM), got Err: {e}")
        });
        assert!(
            group
                .statements
                .iter()
                .any(|s| s.sql.contains("_smelt_output_clamp")),
            "expected the output clamp wrapper in the emitted statements: {:?}",
            group.statements
        );
    }
}
