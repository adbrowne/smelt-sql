//! The shared, pure model-diagnostics builder (`docs/specs/ui_model_diagnostics.md`
//! §Surface "smelt-runtime builder"): the single place a model's full derived
//! state — property set, relation contract, per-cell technique previews — is
//! assembled. `smelt-cli`'s `explain` report and `smelt-ui`'s diagnostics
//! endpoint are both thin, read-only renderers over [`ModelDiagnostics`]; they
//! must not re-derive any of this data themselves
//! (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer boundary").
//!
//! This module operates purely over already-resolved facts (a [`ModelFile`],
//! its upstream models and sources, a caller-built [`BoundContext`]) — it
//! never touches a live backend or the maintenance ledger
//! (`docs/specs/ui_model_diagnostics.md` §Constraints).
//!
//! The per-cell technique-preview set (`PlanCellDiagnostics`) is added in a
//! later stage of the diagnostics builder; this module currently populates
//! the property set and relation contract halves of [`ModelDiagnostics`] and
//! carries an always-empty `cells` field as a typed placeholder.

use std::collections::BTreeMap;

use serde::Serialize;

use smelt_core::config::{Grain as ContractGrain, TimeseriesConfig};
use smelt_core::{Granularity, ModelFile, SourceInfo};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult};
use smelt_logical::analysis::walk::{
    model_property_vector, ColumnComparability, ColumnDeterminism, ColumnDiscriminant, DerivedFd,
    Grain as PropertyGrain,
};
use smelt_logical::maintenance::choice::technique_requires_row_identity;
use smelt_logical::maintenance::derive::row_identity;
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, emit_in_place_update, emit_keyed_fold,
    emit_per_group_recompute, MaintenanceDialect, MaintenanceStatement, Region, StatementGroup,
};
use smelt_logical::maintenance::{PlanCell, RowIdentity, RowIdentityVerdict, Technique, Trigger};
use smelt_planner::SourceTimeseriesMap;

use crate::compile::{CompilerRegistry, EphemeralResolver};
use crate::cumulative::classify_cumulative_sql;
use crate::transformer::{inject_source_filters, inject_time_filter, SourceBound, TimeRange};

/// Errors the diagnostics builder can surface. Fail-loud
/// (`CLAUDE.md` §"Fail-loud discipline"): a model whose SQL cannot be
/// classified into a property vector is reported as an error, never silently
/// defaulted to an empty/optimistic property set.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error("could not derive the property set for model {model}: SQL did not parse into an analyzable query tree")]
    PropertyDerivation { model: String },
}

/// The model's full derived-property set (`docs/specs/model_properties.md`
/// §Surface), serialized from the existing single-owner walk output
/// ([`smelt_logical::analysis::walk::model_property_vector`]) plus the two
/// other already-derived, single-call facts reachable at whole-model scope:
/// region row identity ([`row_identity`]) and per-source bound/reach
/// ([`derive_model_bounds`]). This struct never re-derives any of these
/// facts — it is an adapter over the existing walk/derive outputs, adding
/// `Serialize` and giving the composed shape one name.
///
/// Scope note: `model_properties.md` §Surface catalogues properties beyond
/// what is folded into a single top-level `PropertyVector`/bound-map call —
/// several (event-time monotonicity trace, partition alignment, fan-out/
/// cardinality, skeleton-role extraction, …) are scope-, join-, or
/// column-position facts the walk computes internally per node but does not
/// yet expose as a single whole-model derivation, and several catalogue rows
/// are themselves `not-yet`/`partial` maturity in the spec. `PropertySet`
/// covers every `built`-maturity property reachable from one already-derived
/// per-model call; extending it to the remaining catalogue rows needs new
/// plumbing to locate their inputs (e.g. the event-time expression's AST
/// node) and is left to a follow-up phase rather than invented here.
#[derive(Debug, Clone, Serialize)]
pub struct PropertySet {
    /// Output columns of the model, in projection order.
    pub columns: Vec<String>,
    /// The proven grain (keys). Empty ⇒ unkeyed
    /// (`model_properties.md` §"Fan-out / cardinality" — the grain a
    /// conditional write's row identity is built from, see `row_identity`
    /// below).
    pub grain: PropertyGrain,
    /// Query-derived functional dependencies (`model_properties.md` — the
    /// FD set implied by grain + literal columns).
    pub functional_dependencies: Vec<DerivedFd>,
    /// Per-column determinism (`model_properties.md` §"Determinism (run vs
    /// row) and the nondeterminism predicate").
    pub determinism: Vec<ColumnDeterminism>,
    /// Per-column change-comparability (`model_properties.md` §"Change
    /// comparability").
    pub comparability: Vec<ColumnComparability>,
    /// Per-column aggregate discriminants (`model_properties.md` §"Algebraic
    /// discriminants").
    pub discriminants: Vec<ColumnDiscriminant>,
    /// Output columns that are constant literals here, name → literal text.
    pub literal_columns: Vec<(String, String)>,
    /// Whether an output column crosses a set operation whose branches are
    /// not proven key-disjoint — a structural barrier for FD survival.
    pub has_set_op_barrier: bool,
    /// Whether an input join proves `OneToMany` (row-multiplying).
    pub has_fan_out_join: bool,
    /// The model's own region row identity (`model_properties.md` §"Region
    /// row identity"): declared `unique_key` → proven grain key → the
    /// identity-free `WholeRow` fallback.
    pub row_identity: RowIdentityVerdict,
    /// Per-upstream-source bound/reach (`model_properties.md` §"Unified
    /// bound / reach derivation"), keyed by source name.
    pub source_bounds: BTreeMap<String, BoundResult>,
}

impl PropertySet {
    /// Derive a model's [`PropertySet`] from its (frontmatter-stripped) SQL,
    /// its declared `unique_key`, and a caller-built [`BoundContext`]
    /// (mirroring `smelt-cli::explain::compute_source_bounds`'s own
    /// construction: one `BoundContext::add_source` per upstream source with
    /// a declared timeseries clock).
    fn derive(
        model_name: &str,
        sql: &str,
        declared_unique_key: &[String],
        bound_ctx: &BoundContext,
    ) -> Result<Self, DiagnosticsError> {
        let vector = model_property_vector(sql, &JoinContext::new()).ok_or_else(|| {
            DiagnosticsError::PropertyDerivation {
                model: model_name.to_string(),
            }
        })?;
        let identity = row_identity(declared_unique_key, sql);
        let source_bounds: BTreeMap<String, BoundResult> =
            derive_model_bounds(sql, bound_ctx).into_iter().collect();

        Ok(PropertySet {
            columns: vector.columns,
            grain: vector.grain,
            functional_dependencies: vector.fds,
            determinism: vector.determinism,
            comparability: vector.comparability,
            discriminants: vector.discriminants,
            literal_columns: vector.literal_columns,
            has_set_op_barrier: vector.has_set_op_barrier,
            has_fan_out_join: vector.has_fan_out_join,
            row_identity: identity,
            source_bounds,
        })
    }
}

/// The clock slot's shared fields (`docs/specs/models.md` §"The Relation
/// Contract": clock and identity "carry identical field paths" across both
/// providers). Rendered identically whichever provider filled it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractClock {
    pub event_time_column: String,
    pub partition_column: String,
    pub granularity: Granularity,
}

/// One provider's fill of the Relation Contract's **declared-and-checked**
/// shape-defining slots — the clock and identity — plus the derived `grain`
/// label that summarizes them (`docs/specs/models.md` §"Refresh axis",
/// §"The Relation Contract"; `docs/specs/sources.md` §"The source as a
/// Relation Contract provider"). Both a source and a model output are
/// rendered through this one struct — a consumer never needs to know which
/// provider filled it (`clock`/`identity` are the two fields every provider
/// fills through the same field paths).
///
/// `clock`/`identity`/`derived_grain` are all `None` for a provider that
/// declares neither fact — legal for a source (no admission gate to fail),
/// reported rather than refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<RelationContractClock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_grain: Option<ContractGrain>,
}

impl RelationContractView {
    /// Provider-agnostic construction from the raw declared facts — reads
    /// `Option<&TimeseriesConfig>` / `Option<&[String]>` directly rather than
    /// a `SourceInfo` or `ModelMetadata`, so there is exactly one derivation
    /// reused by both providers (`smelt_core::config::derive_grain`), never a
    /// source-specific or model-specific reimplementation.
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

/// Assemble a model's own [`RelationContractView`] plus its inbound edges'
/// contracts (`docs/specs/models.md` §"The Relation Contract").
///
/// Model-to-model edges come from `model_upstream` (`DependencyGraph::
/// get_upstream`) — resolved against `models` for each upstream's own
/// declared clock/identity. Source edges are read directly off `model`'s own
/// `smelt.sources.*` refs (`model.refs`), because the graph layer's
/// dependency map excludes per-entity source refs entirely
/// (`smelt_core::graph::DependencyGraph::build` filters `first == "sources"`
/// out of `deps`) — the graph layer's model/source distinction, not a second
/// one invented here. Edges are sorted by name for a deterministic report.
///
/// This is the single owner of relation-contract derivation
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer
/// boundary"): `smelt-cli`'s `explain` report calls this function rather
/// than deriving the contract itself.
pub fn build_relation_contract(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[SourceInfo],
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

/// The full closed technique registry the technique-preview set enumerates,
/// one entry per [`Technique`] variant (`docs/specs/ui_model_diagnostics.md`
/// §Semantics "Technique preview set": "never partial by omission"). There
/// is no separate "region recompute" registry member: `Technique::
/// DeleteInsert` **is** region recompute — the DELETE-the-window/INSERT-its-
/// recompute op, named twice in the spec prose (once as the family a cell
/// may itself admit for a `RecomputeRegion` corner, once as the
/// always-available fallback every other cell's preview set offers) but a
/// single emitted-statement shape and a single [`Technique`] value.
const ALL_TECHNIQUES: [Technique; 5] = [
    Technique::DeleteInsert,
    Technique::KeyedFold,
    Technique::ColumnScopedMerge,
    Technique::InPlaceUpdate,
    Technique::PerGroupRecompute,
];

/// One statement of a [`TechniquePreview`]'s illustrative SQL — a
/// `Serialize`-able projection of [`smelt_logical::maintenance::emit::
/// MaintenanceStatement`] (which does not itself derive `Serialize`, kept
/// out of `smelt-logical`'s dependency-free emit module by design).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewStatement {
    pub sql: String,
}

/// A preview entry's admissibility verdict
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility
/// verdict"). Exactly one entry per cell is `Admitted`; a `NotApplicable`
/// entry always carries a non-empty `reason` (fail-loud discipline,
/// `CLAUDE.md` §"Fail-loud discipline").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Admissibility {
    /// This is the technique the derived plan actually resolved for the
    /// cell (`cell.technique`) — its statements are byte-identical to what
    /// a live run executing this cell would emit
    /// (`docs/specs/incremental_models.md` §"Statement emission (single
    /// owner)").
    Admitted,
    /// Proven sound for this cell by the same admission logic that governs
    /// real execution (`smelt_logical::maintenance::choice`), but not the
    /// technique the plan resolved. Today this is exactly `Technique::
    /// DeleteInsert` (region recompute) whenever it is not itself the
    /// admitted technique — always sound, contract-agnostic over replayable
    /// input (`docs/specs/ui_model_diagnostics.md` §Semantics
    /// "Admissibility verdict").
    InterchangeableAlternative,
    /// This technique's structural preconditions are not met for this cell.
    /// The emitter is still called against the cell's own contract/
    /// identity/column data to render illustrative SQL where that is
    /// possible — `reason` makes clear the rendered SQL is not a real
    /// option for this cell, never silently indistinguishable from
    /// `InterchangeableAlternative`.
    NotApplicable { reason: String },
}

/// One technique's rendered SQL for one cell, plus its admissibility
/// verdict (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime
/// builder"; §Semantics "Technique preview set").
#[derive(Debug, Clone, Serialize)]
pub struct TechniquePreview {
    pub technique: Technique,
    /// Whether `statements` must run inside a single backend transaction
    /// (`smelt_logical::maintenance::emit::StatementGroup::transactional`).
    /// `false` (with an empty `statements`) when the technique's statements
    /// could not be built at all for this cell.
    pub transactional: bool,
    pub statements: Vec<PreviewStatement>,
    pub admissibility: Admissibility,
}

/// A maintenance cell's identifying fields plus its technique-preview set
/// (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder";
/// §Semantics "Technique preview set").
#[derive(Debug, Clone, Serialize)]
pub struct PlanCellDiagnostics {
    /// The cell's column group name (`smelt_logical::maintenance::PlanCell::group`).
    pub group: String,
    /// `{:?}`-rendered `smelt_logical::maintenance::Trigger` — mirrors the
    /// existing CLI report's own rendering (`explain::
    /// build_maintenance_plan_report`), not a re-derivation.
    pub trigger: String,
    /// `{:?}`-rendered `smelt_logical::maintenance::Corner`.
    pub corner: String,
    /// The technique the derived plan actually resolved for this cell —
    /// the same value as the one `technique_previews` entry carrying
    /// [`Admissibility::Admitted`].
    pub admitted_technique: Technique,
    /// The cell's own region row identity (P2,
    /// `model_properties.md` §"Region row identity").
    pub row_identity: RowIdentity,
    /// One entry per [`ALL_TECHNIQUES`] member — never partial by omission.
    pub technique_previews: Vec<TechniquePreview>,
}

/// Build one technique's illustrative [`StatementGroup`] for `cell`, using
/// the same pure emitters a live run uses
/// (`docs/specs/incremental_models.md` §"Statement emission (single
/// owner)") — parameterized by an explicit `technique` argument rather than
/// reading `cell.technique`, so every technique in [`ALL_TECHNIQUES`] can be
/// previewed regardless of which one the cell actually admitted
/// (`docs/specs/ui_model_diagnostics.md` §Design "Why preview *every*
/// technique").
///
/// This is the sole owner of the symbolic (no real `--period` window)
/// statement derivation — `smelt-cli::explain` no longer keeps a second copy
/// (`smelt_cli::explain::build_admitted_statement_group` reads this
/// function's output, verbatim, out of the `Admitted` technique-preview
/// entry rather than re-deriving it). This version always renders the
/// symbolic `{{window_start}}`/`{{window_end}}` output-window placeholders:
/// a technique-preview build is a display-only illustration of a cell's
/// shape, not a `--period`-bound dry run. `smelt-cli`'s own
/// `build_delete_insert_period_statement_group` is the one surviving
/// CLI-side re-derivation, scoped to exactly the one case a real `--period`
/// changes in a way plain token substitution over this function's
/// placeholder output cannot reproduce (`Technique::DeleteInsert`'s skew
/// inversion and widened source-scan pushdown) — every other technique's
/// real-`--period` rendering is plain substitution over this function's own
/// output, not a second derivation.
///
/// Returns `Err(reason)` when this technique's structural preconditions
/// (a declared `timeseries.partition_column` for `DeleteInsert`, a
/// classifiable cumulative shape for `KeyedFold`, a non-empty `unique_key`
/// for `ColumnScopedMerge`, a `Trigger::ColumnAdded` cell for
/// `InPlaceUpdate`) cannot be assembled from `cell`/`model`'s own data —
/// never a fabricated statement group.
#[allow(clippy::too_many_arguments)]
fn build_technique_statements(
    technique: Technique,
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    unique_key: &[String],
    source_timeseries: &SourceTimeseriesMap,
    cell: &PlanCell,
) -> Result<StatementGroup, String> {
    let trigger = &cell.trigger;
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let table_name = format!("{schema}.{}", model.db_name_owned());
    let placeholder_range = || TimeRange {
        start: "{{window_start}}".to_string(),
        end: "{{window_end}}".to_string(),
    };

    match technique {
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

            let wrapped_raw =
                inject_time_filter(&stripped_sql, &partition_col, &placeholder_range())
                    .map_err(|e| format!("failed to inject the output clamp: {e}"))?;

            let compiled = registry
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &wrapped_raw, resolver)
                .map_err(|e| format!("failed to compile model body: {e}"))?;

            let region = Region {
                start: "'{{window_start}}'".to_string(),
                end: "'{{window_end}}'".to_string(),
            };

            Ok(emit_delete_insert(
                &table_name,
                &partition_col,
                &region,
                &compiled.sql,
                dialect,
            ))
        }
        Technique::KeyedFold => {
            let model_has_timeseries = model
                .metadata
                .as_ref()
                .is_some_and(|m| m.timeseries.is_some());
            let declared_fds: &[smelt_core::config::FunctionalDependency] = model
                .metadata
                .as_ref()
                .map(|m| m.functional_dependencies.as_slice())
                .unwrap_or(&[]);
            let classification = classify_cumulative_sql(
                &model.name,
                &stripped_sql,
                source_timeseries,
                model_has_timeseries,
                declared_fds,
            )
            .map_err(|e| format!("{e}"))?;

            // `Technique::KeyedFold` cells are only synthesized for the
            // window-forward run shape today (`derive_new_data`'s
            // `Grain::Key` arm requires a `FoldSpec`, which the
            // plain-overwrite/snapshot-reconcile family does not populate —
            // `docs/plans/20260809-keyed-frontier.md` Phase 3's own scope
            // note). A `None` here would mean this cell reached a
            // snapshot-reconcile model — an internal invariant violation,
            // not a model error.
            let driving_ts = classification
                .driving_source
                .timeseries
                .clone()
                .ok_or_else(|| {
                    "internal error: KeyedFold cell resolved for a snapshot-reconcile model \
                 (no clocked driving source)"
                        .to_string()
                })?;
            let time_range = placeholder_range();
            let mut bound_map = std::collections::HashMap::new();
            bound_map.insert(
                classification.driving_source.name.clone(),
                SourceBound {
                    partition_col: driving_ts.partition_column.clone(),
                    before_secs: 0,
                    after_secs: 0,
                },
            );
            let pushed = inject_source_filters(&stripped_sql, &bound_map, &time_range);

            // State augmentation happens on this RAW, pre-compile SQL, not
            // the compiled/cast-wrapped output — mirrors
            // `smelt-runtime::cumulative::execute_cumulative_aggregate`'s
            // ordering fix (`docs/outcomes/20260809-rung2-state-shapes` row
            // 5): a state expression's `per_partition_expr` needs the
            // model's own raw source columns, only in scope before the
            // compiler's `_smelt_typed` cast wrapper.
            let state_columns = classification.state_columns();
            let pushed = smelt_logical::maintenance::emit::state_augmented_projection(
                &pushed,
                &state_columns,
            )
            .map_err(|_| {
                "failed to append decomposed-state columns: SELECT could not be parsed".to_string()
            })?;

            let compiled = registry
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &pushed, resolver)
                .map_err(|e| format!("failed to compile model body: {e}"))?;

            // Single-owner statement rule: the same fold-expansion emitter
            // the executed `MERGE` uses
            // (`smelt-runtime::cumulative::build_cumulative_merge_sql`), so
            // this preview's fold shape can never diverge from what the
            // real run executes (`docs/outcomes/20260809-rung2-state-shapes`
            // row 7).
            let folds: Vec<(String, String)> = classification
                .aggregator_columns
                .iter()
                .flat_map(smelt_logical::maintenance::emit::expand_aggregator_column_folds)
                .collect();

            Ok(emit_keyed_fold(
                &table_name,
                &classification.unique_key,
                &folds,
                &compiled.sql,
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

            Ok(emit_column_scoped_merge(
                &table_name,
                unique_key,
                &compiled.sql,
                dialect,
            ))
        }
        Technique::InPlaceUpdate => {
            let Trigger::ColumnAdded { columns } = trigger else {
                return Err(
                    "Technique::InPlaceUpdate only serves a Trigger::ColumnAdded cell — this \
                     cell's own trigger is not one"
                        .to_string(),
                );
            };
            if columns.is_empty() {
                return Err(
                    "Trigger::ColumnAdded carries no columns — nothing to backfill".to_string(),
                );
            }
            // Every emitter in this module only assembles plain strings a
            // caller already resolved (this module's own doc comment on
            // `emit_keyed_fold`'s `folds` parameter) — the added columns'
            // defining expressions come straight from the model's own
            // current SQL, the same source
            // `smelt_logical::maintenance::derive::derive_column_added`
            // reads via `column_def_from_sql` to classify each column
            // `PureBackfill` in the first place. A `PureBackfill` verdict
            // means every dependency is an already-stored target column —
            // no upstream compile/read is needed, unlike
            // `ColumnScopedMerge`'s `compiled.sql` above.
            let mut assignments = Vec::with_capacity(columns.len());
            for col in columns {
                let def =
                    smelt_logical::maintenance::derive::column_def_from_sql(&stripped_sql, col)
                        .ok_or_else(|| {
                            format!(
                                "could not resolve added column '{col}''s expression in the \
                                 model's own SQL"
                            )
                        })?;
                assignments.push((col.clone(), def.expr.syntax().text().to_string()));
            }
            Ok(StatementGroup {
                statements: emit_in_place_update(&table_name, &assignments, None)
                    .into_iter()
                    .map(|sql| MaintenanceStatement { sql })
                    .collect(),
                transactional: false,
            })
        }
        Technique::PerGroupRecompute => {
            // The repair family (`docs/specs/incremental_models.md` §"The
            // repair family"), built through the SAME two string builders
            // the live keyed run path uses
            // (`crate::maintenance_driver::repair_affected_keys_select` /
            // `repair_candidate_select`) before handing them to the single
            // owner `emit_per_group_recompute` — so this preview's shape
            // can never diverge from what a real run executes.
            let key: Vec<String> = match &cell.row_identity.identity {
                RowIdentity::Key(k) if !k.is_empty() => k.clone(),
                _ if !unique_key.is_empty() => unique_key.to_vec(),
                _ => {
                    return Err(
                        "no proven row identity (RowIdentity::WholeRow) and no declared \
                         unique_key — per-group recompute recomputes whole key groups and has \
                         no meaning without a group key"
                            .to_string(),
                    );
                }
            };
            // The bounded per-group read slice is admission obligation 4;
            // a cell carrying none has no affected-key read to illustrate.
            let clamp = cell.scans.first().ok_or_else(|| {
                "this cell carries no derived scan clamp — the repair's affected-key read is \
                 bounded by the cell's own proven slice, never by an assumed one"
                    .to_string()
            })?;
            // The mutated source's physical table, named the same way the
            // default source mapping does (`SourceInfo::db_name_for_target`
            // — `<schema>.<address segments joined by _>`, and a
            // `PlanCell`'s clamp carries the bare address `raw.orders`,
            // i.e. the `sources.` prefix already stripped). A preview has
            // no `SourceInfo` to consult, so a `name:` override is not
            // reflected here — this SQL is illustrative and never executed.
            let source_table = format!("{schema}.sources_{}", clamp.source.replace('.', "_"));
            // Typed literals, unlike `DeleteInsert`'s bare quoted
            // placeholders: `widened_scan_predicate` uses the region's
            // endpoints as arithmetic operands, so the substituted text
            // must bind as a timestamp (mirrors the live run path in
            // `crate::execute`).
            let region = Region {
                start: "TIMESTAMP '{{window_start}}'".to_string(),
                end: "TIMESTAMP '{{window_end}}'".to_string(),
            };
            let affected_keys_select = crate::maintenance_driver::repair_affected_keys_select(
                &source_table,
                &key,
                Some(clamp),
                &region,
            );
            // The fold's own create/merge path already carries a decomposed
            // combiner's hidden state columns in the physical table (P10,
            // `docs/outcomes/20260809-repair-family/phases/10-plan.md`) — a
            // repair candidate must widen for them too, or this preview's
            // shape would diverge from what a real run executes. A live
            // `Technique::PerGroupRecompute` cell only ever exists for a
            // model that already classified as a cumulative aggregate
            // (`derive_new_data`'s key-grain narrowing), but this preview
            // can be asked to illustrate the technique against an
            // arbitrary `PlanCell`/model pairing that never classifies —
            // fall back to "no hidden state" rather than refusing the
            // whole preview.
            let model_has_timeseries = model
                .metadata
                .as_ref()
                .is_some_and(|m| m.timeseries.is_some());
            let declared_fds: &[smelt_core::config::FunctionalDependency] = model
                .metadata
                .as_ref()
                .map(|m| m.functional_dependencies.as_slice())
                .unwrap_or(&[]);
            let state_columns = classify_cumulative_sql(
                &model.name,
                &stripped_sql,
                source_timeseries,
                model_has_timeseries,
                declared_fds,
            )
            .map(|c| c.state_columns())
            .unwrap_or_default();
            let augmented_sql = crate::maintenance_driver::repair_augmented_model_sql(
                &stripped_sql,
                &state_columns,
            )
            .map_err(|e| format!("{e}"))?;
            let compiled = registry
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &augmented_sql, resolver)
                .map_err(|e| format!("failed to compile model body: {e}"))?;
            let candidate_select = crate::maintenance_driver::repair_candidate_select(
                &compiled.sql,
                &key,
                &affected_keys_select,
            );
            Ok(emit_per_group_recompute(
                &table_name,
                &crate::maintenance_driver::repair_staged_relation(&model.db_name_owned()),
                &key,
                &affected_keys_select,
                &candidate_select,
                dialect,
            ))
        }
    }
}

/// Resolve one technique preview entry's [`Admissibility`], given `cell`'s
/// own admitted technique/row-identity and `technique`'s already-attempted
/// build outcome ([`build_technique_statements`]'s result, or an
/// equivalent caller-supplied one for a synthetic/unit-test cell).
///
/// Pure and read-only: consults `cell.technique` (equality only, never
/// re-derived) and [`technique_requires_row_identity`] (the new additive
/// `choice.rs` classifier) — never [`smelt_logical::maintenance::choice::
/// resolve_cell_choice`] itself, so this function cannot influence real
/// execution admission (`docs/specs/ui_model_diagnostics.md` §Design "Why
/// preview *every* technique").
fn resolve_technique_admissibility(
    cell: &PlanCell,
    technique: Technique,
    build_result: &Result<StatementGroup, String>,
) -> Admissibility {
    if technique == cell.technique {
        return Admissibility::Admitted;
    }
    if technique == Technique::DeleteInsert {
        // Region recompute is always sound and contract-agnostic over
        // replayable input when it is not itself the admitted technique
        // (`docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility
        // verdict") — unless this specific cell's model cannot even build
        // the illustrative statements (no declared partition axis), in
        // which case there is nothing to be interchangeable with.
        return match build_result {
            Ok(_) => Admissibility::InterchangeableAlternative,
            Err(reason) => Admissibility::NotApplicable {
                reason: reason.clone(),
            },
        };
    }
    if technique_requires_row_identity(technique)
        && matches!(cell.row_identity.identity, RowIdentity::WholeRow)
    {
        return Admissibility::NotApplicable {
            reason: format!(
                "no proven row identity (RowIdentity::WholeRow) for this cell — {technique:?} \
                 addresses rows individually and requires a key"
            ),
        };
    }
    if let Err(reason) = build_result {
        return Admissibility::NotApplicable {
            reason: reason.clone(),
        };
    }
    // Building succeeded, but `smelt_logical::maintenance::choice`'s own
    // admission logic never proves two different targeted-write technique
    // families interchangeable for the same cell — a derived plan admits
    // exactly one family per cell (`choice.rs`'s own module doc comment:
    // "the resolvable set" is `{the cell's own admitted technique,
    // RegionRecompute}`), so any other family is structurally not a real
    // option here even when the emitter itself could render SQL for it.
    Admissibility::NotApplicable {
        reason: format!(
            "this cell's derived plan resolved {:?}, not {technique:?} — smelt_logical::\
             maintenance::choice's admission logic does not prove different targeted-write \
             technique families interchangeable for the same cell",
            cell.technique
        ),
    }
}

/// Build one cell's [`PlanCellDiagnostics`]: one [`TechniquePreview`] per
/// [`ALL_TECHNIQUES`] member, never partial by omission
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview
/// set").
#[allow(clippy::too_many_arguments)]
pub fn build_plan_cell_diagnostics(
    cell: &PlanCell,
    model: &ModelFile,
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    unique_key: &[String],
    source_timeseries: &SourceTimeseriesMap,
) -> PlanCellDiagnostics {
    let technique_previews = ALL_TECHNIQUES
        .into_iter()
        .map(|technique| {
            let build_result = build_technique_statements(
                technique,
                model,
                schema,
                target,
                registry,
                resolver,
                dialect,
                unique_key,
                source_timeseries,
                cell,
            );
            let (statements, transactional) = match &build_result {
                Ok(group) => (
                    group
                        .statements
                        .iter()
                        .map(|s| PreviewStatement { sql: s.sql.clone() })
                        .collect(),
                    group.transactional,
                ),
                Err(_) => (Vec::new(), false),
            };
            let admissibility = resolve_technique_admissibility(cell, technique, &build_result);
            TechniquePreview {
                technique,
                transactional,
                statements,
                admissibility,
            }
        })
        .collect();

    PlanCellDiagnostics {
        group: cell.group.clone(),
        trigger: format!("{:?}", cell.trigger),
        corner: format!("{:?}", cell.corner),
        admitted_technique: cell.technique,
        row_identity: cell.row_identity.identity.clone(),
        technique_previews,
    }
}

/// A model's full derived state (`docs/specs/ui_model_diagnostics.md`
/// §Surface "smelt-runtime builder"): its property set, its own relation
/// contract plus every inbound edge's, and its maintenance plan's per-cell
/// technique previews. The single value both `smelt-cli`'s `explain` report
/// and `smelt-ui`'s diagnostics endpoint render, verbatim, never re-deriving
/// any field themselves.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDiagnostics {
    pub model: String,
    pub properties: PropertySet,
    pub contract: RelationContractView,
    pub inbound_edges: Vec<InboundEdgeContract>,
    pub cells: Vec<PlanCellDiagnostics>,
}

/// Build a model's [`ModelDiagnostics`] — the property set, relation
/// contract, and per-cell technique-preview set
/// (`docs/specs/ui_model_diagnostics.md` §Surface).
///
/// Pure over already-resolved inputs (Salsa purity rule,
/// `docs/specs/architecture.md` §"Salsa purity rule (analysis)"): every
/// argument is already-derived data a caller assembles from its own
/// Salsa/discovery queries (the same shape `smelt-cli::explain`'s existing
/// report builders already take), so this function never touches a live
/// backend or the maintenance ledger
/// (`docs/specs/ui_model_diagnostics.md` §Constraints). `plan_cells` is the
/// model's already-derived `MaintenancePlan::cells` (empty for a model with
/// no maintenance plan — `full`-mode or a view); `schema`/`target`/
/// `registry`/`resolver`/`dialect`/`source_timeseries` are the same
/// already-resolved compilation facts `smelt-cli::explain`'s `--show-sql`
/// path assembles before calling the (moving) statement-group builder.
///
/// `write_unique_key` is a second, deliberately distinct unique-key input:
/// the effective *write*/MERGE-dedup key `Technique::ColumnScopedMerge`'s
/// preview must key on — `Config::get_incremental_with_metadata`'s
/// config-merged `batched.unique_key` (frontmatter wins wholesale over a
/// `smelt.yml` model override; the *only* surviving spelling for a
/// row-shaped, non-clocked join whose own `.sql` frontmatter cannot carry a
/// top-level `unique_key:` — `docs/specs/models.md` §"The Relation
/// Contract"). This is never the same fact as the model's own top-level
/// `unique_key:` (`model.metadata.unique_key`, read below as
/// `declared_unique_key`): that one is the *identity*/grain fact `row_identity`
/// derives P2 region-row-identity from, and is empty for exactly the
/// row-shaped models whose write key only exists via the `batched:` override
/// (`examples/timeseries/models/daily_events_enriched.sql`'s own doc
/// comment). Passing `declared_unique_key` to both purposes — the shape this
/// function used before this parameter existed — silently starved every such
/// model's `ColumnScopedMerge` preview of its real key.
#[allow(clippy::too_many_arguments)]
pub fn build_model_diagnostics(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[SourceInfo],
    bound_ctx: &BoundContext,
    plan_cells: &[PlanCell],
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    source_timeseries: &SourceTimeseriesMap,
    write_unique_key: &[String],
) -> Result<ModelDiagnostics, DiagnosticsError> {
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let declared_unique_key: Vec<String> = model
        .metadata
        .as_deref()
        .and_then(|m| m.unique_key.clone())
        .unwrap_or_default();

    let properties = PropertySet::derive(
        &model.canonical_path(),
        &stripped_sql,
        &declared_unique_key,
        bound_ctx,
    )?;

    let (contract, inbound_edges) =
        build_relation_contract(model, models, model_upstream, source_infos);

    let cells = plan_cells
        .iter()
        .map(|cell| {
            build_plan_cell_diagnostics(
                cell,
                model,
                schema,
                target,
                registry,
                resolver,
                dialect,
                write_unique_key,
                source_timeseries,
            )
        })
        .collect();

    Ok(ModelDiagnostics {
        model: model.canonical_path(),
        properties,
        contract,
        inbound_edges,
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::{Config, Materialization, Target};
    use smelt_core::metadata::ModelMetadata;
    use smelt_core::{Granularity, RefInfo, SmeltRef};
    use std::collections::HashMap;

    /// A minimal `Trigger::Backfill` cell — `build_technique_statements`
    /// reads only the trigger off it for the arms this module unit-tests.
    fn synthetic_backfill_cell() -> PlanCell {
        PlanCell {
            group: "{*}".to_string(),
            trigger: Trigger::Backfill,
            corner: smelt_logical::maintenance::Corner::RecomputeRegion,
            technique: Technique::DeleteInsert,
            partition_local: smelt_logical::maintenance::PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: Default::default(),
            key_scope: None,
            recompute_fallback: None,
        }
    }

    fn duckdb_target() -> Target {
        Target {
            target_type: "duckdb".to_string(),
            database: Some("test.duckdb".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        }
    }

    fn test_config() -> Config {
        let mut targets = HashMap::new();
        targets.insert("dev".to_string(), duckdb_target());
        Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
            target: None,
            state: Default::default(),
            maintenance: None,
            probes: Default::default(),
        }
    }

    /// `build_technique_statements`'s `Technique::DeleteInsert` branch must
    /// succeed for a model whose outermost FROM is a `TableExpr`-returning
    /// function call (`smelt.functions.windowed(...)`) — the shape
    /// `silver.sessions` uses in `examples/web_analytics`
    /// (`crates/smelt-cli/tests/explain_model.rs::sessions_show_sql_emits_statements`
    /// is the real-`--period` counterpart of this unit test). Clamping the
    /// raw, unexpanded model body *before* compiling — never the compiled
    /// output, which the compiler's own printer has already expanded — is
    /// what makes this succeed: `inject_time_filter`'s reparse only ever
    /// sees a plain FROM clause referencing a CTE, never the
    /// function-expansion's reparse-hostile output.
    ///
    /// Ported from `smelt-cli::explain`'s own regression test of the same
    /// name when the CLI's duplicate no-`--period` derivation was retired in
    /// favor of this shared builder
    /// (`docs/plans/20260725-ui-model-diagnostics.md`).
    #[test]
    fn delete_insert_clamp_succeeds_on_table_expr_function_from() {
        use smelt_core::config::TimeseriesConfig;

        let content = "SELECT device_id, d FROM smelt.functions.windowed(src => raw_events)";
        let path: std::path::PathBuf = "sessions.sql".into();
        let model = ModelFile {
            name: "sessions".to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: content.to_string(),
            refs: vec![RefInfo {
                has_named_params: false,
                range: Default::default(),
                smelt_ref: SmeltRef::Path(vec!["raw_events".to_string()]),
            }],
            parse_errors: Vec::new(),
            metadata: Some(Box::new(ModelMetadata {
                materialization: Some(Materialization::Table),
                timeseries: Some(TimeseriesConfig {
                    event_time_column: "d".to_string(),
                    partition_column: "d".to_string(),
                    granularity: Granularity::Day,
                    week_start: None,
                    assert_monotonic: false,
                }),
                ..Default::default()
            })),
            kind: smelt_core::ModelKind::Sql,
            address_segments: vec!["sessions".to_string()],
        };

        let config = test_config();
        let mut registry = CompilerRegistry::new(&config, &config.targets);
        let mut fn_bodies: crate::fn_bodies::FnBodyMap = HashMap::new();
        fn_bodies.insert(
            "windowed".to_string(),
            (
                vec![("src".to_string(), None)],
                "(SELECT * FROM src)".to_string(),
            ),
        );
        registry.set_function_bodies_all(fn_bodies);
        let resolver = registry.get("dev").build_ephemeral_resolver(&[], "main");

        let source_timeseries = SourceTimeseriesMap::new();

        let group = build_technique_statements(
            Technique::DeleteInsert,
            &model,
            "main",
            "dev",
            &registry,
            &resolver,
            MaintenanceDialect::DuckDb,
            &[],
            &source_timeseries,
            &synthetic_backfill_cell(),
        )
        .unwrap_or_else(|e| {
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
