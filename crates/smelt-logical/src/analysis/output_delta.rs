//! Output-delta shape — the shape of change a model *emits* per column group.
//!
//! See `docs/specs/model_properties.md` §"Output-delta shape". A distinct
//! question from input-delta discovery (`analysis::input_delta`: which rows
//! of an *input* are new) or affected-key discovery (which keys a delta
//! touches): this proof asks what shape a model's *output* takes when one of
//! its inputs changes. The verdict is the three-level lattice [`OutputDelta`]
//! (`AppendOnlyWindow ⊑ KeyedUpsert ⊑ General`), produced by the shared
//! composition walk ([`crate::analysis::walk`]) via [`OutputDeltaTransfer`]
//! and folded to one verdict per column group by [`derive_output_delta`].
//!
//! **Leaf seeding mirrors `input_delta_discovery`'s fail-closed pattern**
//! (`analysis::input_delta`): a base relation's shape comes from the
//! source's declared mutation profile, and an undeclared/unrecognised
//! profile never yields an optimistic shape — it falls back to `General`,
//! never `AppendOnlyWindow`/`KeyedUpsert`.

use std::collections::{BTreeMap, BTreeSet};

use smelt_parser::{ColumnRef, Expr};

use crate::analysis::expr_util::collect_column_refs;
pub use crate::analysis::input_delta::MutationProfile;
use crate::analysis::join_shape::{fan_out, Cardinality, JoinContext};
use crate::analysis::walk::{
    resolve_alias_source, walk, InputItem, LeafInput, NodeCx, OpNode, QueryTree, RelationSource,
    SelectNode, SetOpKind, SetOpNode, Transfer,
};
use crate::analysis::{
    item_alias, item_expr, resolve_scope_group_by, select_stmt_items, SelectItemKind,
};
use crate::maintenance::grouping::derive_column_groups;
use crate::maintenance::{
    ColumnGroup, MutationProfile as MaintenanceMutationProfile,
    SourceFacts as MaintenanceSourceFacts,
};

/// The output-delta shape lattice (`docs/specs/model_properties.md`
/// §"Output-delta shape"): the shape of change a model emits when one of its
/// inputs changes, ordered by *addressability* — `AppendOnlyWindow`
/// (narrowest: every change lands as new rows within a bounded window) ⊑
/// `KeyedUpsert` (a change revises the row identified by a key set) ⊑
/// `General` (widest: neither addressing holds, arbitrarily rewritten).
/// Degrade-only: [`OutputDelta::meet`] never recovers a narrower shape than
/// either input.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputDelta {
    /// Every emitted change lands as new rows within a bounded window on the
    /// named output axis, never revising an already-emitted row.
    AppendOnlyWindow { axis: String },
    /// A change instead revises the row identified by `keys`, addressable by
    /// that key set rather than by position.
    KeyedUpsert { keys: Vec<String> },
    /// Neither addressing holds — a consumer can only treat the column group
    /// as arbitrarily rewritten. `reason` names the construct or world-fact
    /// that forced the degrade (the fail-closed default).
    General { reason: String },
}

impl OutputDelta {
    /// Lattice position: 0 = narrowest (`AppendOnlyWindow`), 2 = widest
    /// (`General`). Ordered by addressability, not information content.
    pub fn rank(&self) -> u8 {
        match self {
            OutputDelta::AppendOnlyWindow { .. } => 0,
            OutputDelta::KeyedUpsert { .. } => 1,
            OutputDelta::General { .. } => 2,
        }
    }

    /// Degrade-only meet: the less-addressable (higher-rank) of the two
    /// shapes wins. A tie (equal rank) keeps `self` — composition never
    /// needs to distinguish between two shapes of the same rank, only ever
    /// widen past a narrower one.
    pub fn meet(self, other: OutputDelta) -> OutputDelta {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// Per-source facts [`OutputDeltaTransfer`]'s leaf-seeding reads — mirrors
/// `analysis::input_delta::SourceShape`'s fail-closed pattern
/// (`docs/specs/sources.md`), extended with the two extra facts leaf-seeding
/// needs beyond input-delta discovery: the append-only axis (the declared
/// clock/partition column an `AppendOnlyWindow` shape addresses) and the
/// change-feed delta identity (the key set a `KeyedUpsert` shape addresses).
#[derive(Debug, Clone, Default)]
pub struct SourceFacts {
    /// Name as it appears in the model SQL's `smelt.sources.<name>` /
    /// `smelt.models.<name>` ref.
    pub name: String,
    /// The source's declared clock/partition column, when it has one
    /// (`sources.md`'s `timeseries:` presence).
    pub axis: Option<String>,
    /// The declared `mutation_profile.kind` (`sources.md`); `None` is the
    /// undeclared/unknown case — the fail-closed default.
    pub mutation_profile: Option<MutationProfile>,
    /// `change_feed`'s declared per-delta identity column(s)
    /// (`sources.md`'s `delta_identity`).
    pub delta_identity: Option<Vec<String>>,
}

impl SourceFacts {
    /// Build the [`SourceFacts`] leaf-seeding reads from a source's
    /// catalogued `SourceInfo` (`sources.md`), mirroring
    /// `input_delta::SourceShape::from_source_info`'s fail-closed pattern:
    /// `axis` from `timeseries:` presence (`None` when the source declares
    /// no clock), `mutation_profile` from the declared `mutation_profile:`
    /// key (`None` when undeclared — the fail-closed default is unchanged),
    /// `delta_identity` from that block's own declared identity columns.
    /// `name` is not carried on `SourceInfo` itself (it is the bare address
    /// segment the caller already resolved the ref against), so it is taken
    /// as a separate argument — the same shape `smelt-db`'s own
    /// `source_facts(name, info, ..)` adapter uses for the maintenance-layer
    /// `SourceFacts`.
    pub fn from_source_info(name: &str, info: &smelt_core::sources::SourceInfo) -> Self {
        SourceFacts {
            name: name.to_string(),
            axis: info.timeseries.as_ref().map(|t| t.partition_column.clone()),
            mutation_profile: info.mutation_profile.as_ref().map(|m| m.kind.into()),
            delta_identity: info
                .mutation_profile
                .as_ref()
                .and_then(|m| m.delta_identity.clone()),
        }
    }
}

/// The walk verdict: per-output-column shape, in projection order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputDeltaFacts {
    pub columns: Vec<(String, OutputDelta)>,
}

/// The output-delta transfer function (`docs/specs/model_properties.md`
/// §"Output-delta shape" transfer-rule table), run over the shared
/// composition walk (`crate::analysis::walk`).
pub struct OutputDeltaTransfer<'a> {
    pub ctx: &'a JoinContext,
    pub sources: &'a [SourceFacts],
    /// A referenced model's own already-derived per-output-column
    /// output-delta facts, keyed by bare model name (case-insensitive) — the
    /// model-reference leaf case (`model_properties.md` §"Output-delta
    /// shape"). Per output *column*, not a scalar per model
    /// (`docs/outcomes/20260809-output-delta-typing/outcome.md`
    /// 2026-08-09 decision): a scalar would meet-fold a mixed-shape
    /// upstream to its worst group, which the per-column-group decision
    /// rejects. [`derive_workspace_output_deltas`] folds this map across
    /// the real workspace; an absent model or absent column falls back to
    /// `General`, never an optimistic guess.
    pub model_verdicts: &'a BTreeMap<String, OutputDeltaFacts>,
}

/// Leaf-seeding rule (`model_properties.md` §"Output-delta shape",
/// base-relation row): `append_only` with a declared clock ⇒
/// `AppendOnlyWindow`; `change_feed` with a `delta_identity` ⇒
/// `KeyedUpsert`; everything else ⇒ `General`, fail-closed — mirrors
/// `input_delta::input_delta_discovery`'s fail-closed default. A free
/// function (not a method) so a caller with only a source's [`SourceFacts`]
/// in hand — no SQL to walk, e.g. a raw `sources.*` propagation-edge
/// upstream — can seed its shape directly.
/// Normalizes a model-reference name to the `model_verdicts` key: lowercase,
/// with an optional leading `models.` breadcrumb stripped (mirrors the
/// breadcrumb handling in `analysis::fingerprint::relation_matches_source`)
/// — applied on both the insert side ([`derive_workspace_output_deltas`])
/// and the lookup side ([`OutputDeltaTransfer::seed_for_leaf_name`] /
/// [`OutputDeltaTransfer::seed_for_model_column`]) so a `smelt-db`-built key
/// of `models.<addr>` and a runtime-built bare `<addr>` key land on the same
/// entry regardless of which side of the fold spells the breadcrumb.
fn normalize_model_key(name: &str) -> String {
    name.strip_prefix("models.")
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// The fail-closed reason for a bare (unprefixed) relation reference that
/// matches neither a declared source nor a model verdict — names both
/// misses rather than guessing which one the author intended
/// (`model_properties.md` §"Output-delta shape", model-reference-leaf
/// paragraph).
fn bare_relation_miss(bare: &str) -> OutputDelta {
    OutputDelta::General {
        reason: format!(
            "relation '{bare}' has no declared mutation profile and no derived model \
             output-delta verdict"
        ),
    }
}

pub fn seed_shape_for_source(facts: &SourceFacts) -> OutputDelta {
    match &facts.mutation_profile {
        Some(MutationProfile::ChangeFeed) => match &facts.delta_identity {
            Some(keys) if !keys.is_empty() => OutputDelta::KeyedUpsert { keys: keys.clone() },
            _ => OutputDelta::General {
                reason: format!(
                    "source '{}' declares change_feed but no delta_identity",
                    facts.name
                ),
            },
        },
        Some(MutationProfile::AppendOnly) => match &facts.axis {
            Some(axis) => OutputDelta::AppendOnlyWindow { axis: axis.clone() },
            None => OutputDelta::General {
                reason: format!(
                    "source '{}' is append_only but declares no clock/axis column",
                    facts.name
                ),
            },
        },
        Some(MutationProfile::Mutable) => OutputDelta::General {
            reason: format!("source '{}' is a mutable snapshot", facts.name),
        },
        None => OutputDelta::General {
            reason: format!("source '{}' declares no mutation_profile", facts.name),
        },
    }
}

/// The whole-source `ColumnGroup` + shape for a declared source
/// (`incremental_models.md` §"The graph layer" → "Typed edges"): a source's
/// declared columns all share one uniform shape (the source-level mutation
/// profile), unlike a model's per-select-item shapes, so this is always
/// exactly zero or one group — empty when the source declares no columns.
pub fn source_output_delta(
    facts: &SourceFacts,
    info: &smelt_core::sources::SourceInfo,
) -> Vec<(ColumnGroup, OutputDelta)> {
    if info.columns.is_empty() {
        return Vec::new();
    }
    let shape = seed_shape_for_source(facts);
    let group = ColumnGroup {
        columns: info.columns.iter().map(|c| c.name.clone()).collect(),
        mutation_sensitivity: BTreeSet::new(),
        membership_sensitivity: BTreeSet::new(),
    };
    vec![(group, shape)]
}

impl OutputDeltaTransfer<'_> {
    /// Resolve a leaf relation's name (as normalized by the walk:
    /// `sources.<name>`, `models.<name>`, or a bare table name) to its
    /// seeded [`OutputDelta`] — the whole-relation shape (a source's
    /// uniform shape, or the meet across a referenced model's own derived
    /// column shapes). A bare name (neither breadcrumb) resolves against a
    /// declared source first, then a model verdict — the same precedence
    /// [`Self::resolve_column_ref_shape`]'s bare-table arm applies, so the
    /// two paths cannot drift (`model_properties.md` §"Output-delta shape",
    /// model-reference-leaf paragraph). Used by [`Transfer::leaf`], which
    /// has no single column to resolve against; per-column resolution
    /// against a referenced model's facts is [`Self::seed_for_model_column`].
    fn seed_for_leaf_name(&self, name: &str) -> OutputDelta {
        if let Some(bare) = name.strip_prefix("sources.") {
            return self.seed_for_source_name(bare);
        }
        if let Some(bare) = name.strip_prefix("models.") {
            return self
                .model_whole_shape(bare)
                .unwrap_or_else(|| OutputDelta::General {
                    reason: format!(
                        "referenced model '{bare}' has no derived output-delta verdict available"
                    ),
                });
        }
        if let Some(facts) = self
            .sources
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
        {
            return seed_shape_for_source(facts);
        }
        self.model_whole_shape(name)
            .unwrap_or_else(|| bare_relation_miss(name))
    }

    /// Looks up `bare` in `model_verdicts` breadcrumb-insensitively on BOTH
    /// sides: the map itself may be keyed either bare (`smelt-runtime`'s
    /// `ModelFile::canonical_path()`) or breadcrumbed (`smelt-db`'s
    /// `model_delta_inputs`, keyed by the ref path as literally spelled) —
    /// [`derive_workspace_output_deltas`] normalizes its own inserts, but a
    /// caller-supplied map (e.g. a test double, or a future producer) is not
    /// guaranteed to. Tries the normalized key first, then the same key with
    /// a `models.` breadcrumb.
    fn lookup_model_verdicts(&self, bare: &str) -> Option<&OutputDeltaFacts> {
        let key = normalize_model_key(bare);
        self.model_verdicts
            .get(&key)
            .or_else(|| self.model_verdicts.get(&format!("models.{key}")))
    }

    /// The meet across a referenced model's own derived per-column shapes —
    /// `None` when no verdict is available for `bare` at all (the caller
    /// supplies its own not-found reason, since an explicit
    /// `models.`-prefixed miss and a bare-fallback miss name the failure
    /// differently).
    fn model_whole_shape(&self, bare: &str) -> Option<OutputDelta> {
        let facts = self.lookup_model_verdicts(bare)?;
        let mut shapes = facts.columns.iter().map(|(_, shape)| shape.clone());
        Some(match shapes.next() {
            None => OutputDelta::General {
                reason: format!("referenced model '{bare}' has no output columns"),
            },
            Some(first) => shapes.fold(first, OutputDelta::meet),
        })
    }

    /// Per-column resolution of a `models.<name>` reference
    /// (`model_properties.md` §"Output-delta shape", model-reference leaf):
    /// an absent model or a column the model's own facts do not carry both
    /// fail closed to `General`, naming what was missing — never an
    /// optimistic guess.
    fn seed_for_model_column(&self, bare: &str, column: &str) -> OutputDelta {
        match self.lookup_model_verdicts(bare) {
            Some(facts) => facts
                .columns
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(column))
                .map(|(_, shape)| shape.clone())
                .unwrap_or_else(|| OutputDelta::General {
                    reason: format!(
                        "referenced model '{bare}' has no derived output-delta shape for \
                         column '{column}'"
                    ),
                }),
            None => OutputDelta::General {
                reason: format!(
                    "referenced model '{bare}' has no derived output-delta verdict available"
                ),
            },
        }
    }

    /// Bare-name per-column resolution (neither `sources.` nor `models.`
    /// breadcrumb): a declared source wins over a same-named model verdict
    /// (mirrors [`Self::seed_for_leaf_name`]'s precedence); a name matching
    /// neither fails closed naming both misses.
    fn resolve_bare_table_column_shape(&self, bare: &str, column: &str) -> OutputDelta {
        if let Some(facts) = self
            .sources
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(bare))
        {
            return seed_shape_for_source(facts);
        }
        match self.lookup_model_verdicts(bare) {
            Some(facts) => facts
                .columns
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(column))
                .map(|(_, shape)| shape.clone())
                .unwrap_or_else(|| OutputDelta::General {
                    reason: format!(
                        "referenced model '{bare}' has no derived output-delta shape for \
                         column '{column}'"
                    ),
                }),
            None => bare_relation_miss(bare),
        }
    }

    fn seed_for_source_name(&self, bare: &str) -> OutputDelta {
        match self
            .sources
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(bare))
        {
            Some(facts) => seed_shape_for_source(facts),
            None => OutputDelta::General {
                reason: format!("relation '{bare}' has no declared mutation profile"),
            },
        }
    }

    /// Whether any join in this scope's FROM clause proves `OneToMany`
    /// (`analysis::join_shape::fan_out`) — the fail-closed default when no
    /// unique key is declared for the joined side.
    fn scope_has_fan_out(&self, sn: &SelectNode) -> bool {
        let Some(from) = sn.select.from_clause() else {
            return false;
        };
        let result = from
            .joins()
            .any(|join| fan_out(&join, self.ctx) == Cardinality::OneToMany);
        result
    }

    /// This scope's `GROUP BY`/`DISTINCT` output keys, when it aggregates.
    /// `DISTINCT` dedups on the whole projected row, so its key set is every
    /// output column; `GROUP BY`'s key set is the grouping expressions'
    /// output names (falling back to the raw key text when no select item
    /// carries that exact expression as its own, e.g. a key not projected).
    fn scope_group_keys(
        &self,
        sn: &SelectNode,
        items: &[SelectItemKind],
        cx: &NodeCx,
    ) -> Option<Vec<String>> {
        if sn.select.is_distinct() {
            let cols: Vec<String> = cx.columns.iter().map(|c| c.output.clone()).collect();
            if !cols.is_empty() {
                return Some(cols);
            }
        }
        let gb_keys = resolve_scope_group_by(&sn.select, items);
        if gb_keys.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(gb_keys.len());
        for key in &gb_keys {
            let named = items.iter().find_map(|item| {
                if item_expr(item).text().trim() == key {
                    let alias = item_alias(item);
                    Some(if alias.is_empty() {
                        key.clone()
                    } else {
                        alias.to_string()
                    })
                } else {
                    None
                }
            });
            out.push(named.unwrap_or_else(|| key.clone()));
        }
        Some(out)
    }

    /// One select item's own shape, from the meet of every column reference
    /// its expression embeds (`model_properties.md`'s Selection/Projection
    /// row: a simple rename is the one-reference case of this same rule).
    /// A reference-free expression (a constant literal, `COUNT(*)`, an
    /// opaque function call with no traceable column) has no source to
    /// attribute a shape to, so it fails closed to `General` rather than
    /// guessing.
    fn resolve_expr_shape(
        &self,
        expr: &Expr,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &OutputDeltaFacts>,
    ) -> OutputDelta {
        let refs = collect_column_refs(expr);
        if refs.is_empty() {
            return OutputDelta::General {
                reason: "expression has no column reference to attribute an output-delta shape \
                         to (a constant literal, COUNT(*), or an opaque function call)"
                    .to_string(),
            };
        }
        let mut acc: Option<OutputDelta> = None;
        for cref in &refs {
            let shape = self.resolve_column_ref_shape(cref, cx, input_props);
            acc = Some(match acc {
                None => shape,
                Some(a) => a.meet(shape),
            });
        }
        acc.unwrap_or_else(|| OutputDelta::General {
            reason: "no resolvable column reference".to_string(),
        })
    }

    fn resolve_column_ref_shape(
        &self,
        cref: &ColumnRef,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &OutputDeltaFacts>,
    ) -> OutputDelta {
        let Some(source_key) = resolve_alias_source(cx, cref.qualifier()) else {
            return OutputDelta::General {
                reason: format!(
                    "column reference '{}' does not resolve to a single FROM-tree alias in \
                     this scope (ambiguous, or unqualified with more than one input)",
                    cref.name()
                ),
            };
        };
        if let Some(inner) = input_props.get(&source_key) {
            return inner
                .columns
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(cref.name()))
                .map(|(_, shape)| shape.clone())
                .unwrap_or_else(|| OutputDelta::General {
                    reason: format!(
                        "column '{}' not found in the resolved input's own output-delta facts",
                        cref.name()
                    ),
                });
        }
        match cx.aliases.get(&source_key) {
            Some(RelationSource::Table(name)) => match name.strip_prefix("models.") {
                Some(bare) => self.seed_for_model_column(bare, cref.name()),
                None => match name.strip_prefix("sources.") {
                    Some(bare) => self.seed_for_source_name(bare),
                    None => self.resolve_bare_table_column_shape(name, cref.name()),
                },
            },
            _ => OutputDelta::General {
                reason: format!(
                    "column reference '{}' resolves to a relation this proof cannot classify",
                    cref.name()
                ),
            },
        }
    }

    fn select_shape(
        &self,
        sn: &SelectNode,
        children: &[OutputDeltaFacts],
        cx: &NodeCx,
    ) -> OutputDeltaFacts {
        let mut input_props: BTreeMap<String, &OutputDeltaFacts> = BTreeMap::new();
        for (input, child) in sn.inputs.iter().zip(&children[sn.ctes.len()..]) {
            match input {
                InputItem::CteRef { name, alias } => {
                    let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                    input_props.insert(key, child);
                }
                InputItem::Derived {
                    alias: Some(alias), ..
                } => {
                    input_props.insert(alias.to_ascii_lowercase(), child);
                }
                _ => {}
            }
        }

        let Some(items) = select_stmt_items(&sn.select) else {
            return OutputDeltaFacts::default();
        };
        let has_fan_out = self.scope_has_fan_out(sn);
        let group_keys = self.scope_group_keys(sn, &items, cx);

        let mut columns = Vec::with_capacity(items.len());
        for item in &items {
            let alias = item_alias(item).to_string();
            let expr = item_expr(item);

            let mut shape = if expr.window_spec().is_some() {
                OutputDelta::General {
                    reason: format!(
                        "'{alias}' is a window-function column — an emitted row's value can \
                         depend on sibling rows outside the addressed window/key"
                    ),
                }
            } else {
                self.resolve_expr_shape(expr, cx, &input_props)
            };

            if has_fan_out {
                shape = OutputDelta::General {
                    reason: "join proven OneToMany (row-multiplying) degrades output-delta \
                             shape to General"
                        .to_string(),
                };
            }

            if let Some(keys) = &group_keys {
                shape = match shape {
                    OutputDelta::AppendOnlyWindow { .. } => {
                        OutputDelta::KeyedUpsert { keys: keys.clone() }
                    }
                    other => other,
                };
            }

            columns.push((alias, shape));
        }
        OutputDeltaFacts { columns }
    }

    fn setop_shape(&self, so: &SetOpNode, children: &[OutputDeltaFacts]) -> OutputDeltaFacts {
        let branches = &children[so.ctes.len()..];
        let is_union_all = !so.ops.is_empty() && so.ops.iter().all(|o| *o == SetOpKind::UnionAll);

        let Some(first) = branches.first() else {
            return OutputDeltaFacts::default();
        };

        if !is_union_all {
            let op_name = so
                .ops
                .first()
                .map(|o| o.as_str())
                .unwrap_or("set operation");
            let columns = first
                .columns
                .iter()
                .map(|(name, _)| {
                    (
                        name.clone(),
                        OutputDelta::General {
                            reason: format!(
                                "'{op_name}' is not a registered output-delta transfer rule — \
                                 only UNION ALL preserves shape across set-operation arms"
                            ),
                        },
                    )
                })
                .collect();
            return OutputDeltaFacts { columns };
        }

        let mut columns = Vec::with_capacity(first.columns.len());
        for (i, (name, shape)) in first.columns.iter().enumerate() {
            let mut acc = shape.clone();
            for branch in &branches[1..] {
                match branch.columns.get(i) {
                    Some((_, s)) => acc = acc.meet(s.clone()),
                    None => {
                        acc = OutputDelta::General {
                            reason: format!(
                                "a UNION ALL arm has fewer output columns than the first arm \
                                 at position {i}"
                            ),
                        }
                    }
                }
            }
            columns.push((name.clone(), acc));
        }
        OutputDeltaFacts { columns }
    }
}

impl Transfer for OutputDeltaTransfer<'_> {
    type Verdict = OutputDeltaFacts;

    fn leaf(&self, leaf: &LeafInput<'_>, _cx: &NodeCx) -> OutputDeltaFacts {
        OutputDeltaFacts {
            columns: vec![("*".to_string(), self.seed_for_leaf_name(leaf.name))],
        }
    }

    fn operator(
        &self,
        op: &OpNode<'_>,
        children: &[OutputDeltaFacts],
        cx: &NodeCx,
    ) -> OutputDeltaFacts {
        match op {
            OpNode::Unsupported { reason } => OutputDeltaFacts {
                columns: vec![(
                    "*".to_string(),
                    OutputDelta::General {
                        reason: format!("unregistered/unnormalizable construct: {reason}"),
                    },
                )],
            },
            OpNode::Select(sn) => self.select_shape(sn, children, cx),
            OpNode::SetOp(so) => self.setop_shape(so, children),
        }
    }
}

fn to_maintenance_source_facts(sources: &[SourceFacts]) -> Vec<MaintenanceSourceFacts> {
    sources
        .iter()
        .map(|s| MaintenanceSourceFacts {
            name: s.name.clone(),
            mutation: match s.mutation_profile {
                Some(MutationProfile::AppendOnly) => MaintenanceMutationProfile::AppendOnly,
                _ => MaintenanceMutationProfile::MutableSnapshot,
            },
            partition_col: s.axis.clone(),
            unique_key: s.delta_identity.clone().unwrap_or_default(),
            allow_full_scan: true,
        })
        .collect()
}

/// Run the composition walk alone, without the [`ColumnGroup`] fold — the
/// per-output-column verdict [`derive_output_delta`]/
/// [`derive_workspace_output_deltas`] both build on. `model_verdicts` is the
/// model-reference leaf's cross-model input (`OutputDeltaTransfer`'s own
/// field); `None` when `sql` does not parse to a walkable `SELECT`.
pub fn derive_output_delta_facts(
    sql: &str,
    ctx: &JoinContext,
    sources: &[SourceFacts],
    model_verdicts: &BTreeMap<String, OutputDeltaFacts>,
) -> Option<OutputDeltaFacts> {
    QueryTree::from_sql(sql).map(|tree| {
        let transfer = OutputDeltaTransfer {
            ctx,
            sources,
            model_verdicts,
        };
        walk(&tree, &transfer)
    })
}

/// Every column group this model's SQL touches via [`derive_column_groups`]
/// — the same partition [`derive_output_delta`]/
/// [`derive_output_delta_with_model_verdicts`] fold shapes onto, exposed
/// standalone for a caller (the propagation-edge builder) that needs a
/// consumer's own groups without also wanting its output-delta shapes. A
/// synthetic group covering exactly `skeleton_columns` is appended when
/// non-empty: `derive_column_groups` deliberately excludes skeleton columns
/// from its own payload partition (creation is shared by every column, so
/// mutation-sensitivity only partitions the payload), but a skeleton column
/// — most commonly a declared `timeseries.partition_column` — is still a
/// real output column a downstream edge's window addressing needs to find
/// "carried" (`maintenance::edge_type::type_edge`'s carriage check); without
/// this synthetic group, that check would always fail closed for the exact
/// column addressing usually survives on.
pub fn derive_consumer_column_groups(
    sql: &str,
    sources: &[SourceFacts],
    skeleton_columns: &BTreeSet<String>,
) -> Vec<ColumnGroup> {
    let maintenance_sources = to_maintenance_source_facts(sources);
    let mut groups = derive_column_groups(sql, &maintenance_sources, skeleton_columns).groups;
    if !skeleton_columns.is_empty() {
        groups.push(ColumnGroup {
            columns: skeleton_columns.iter().cloned().collect(),
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        });
    }
    groups
}

/// Every bare column name referenced anywhere in `sql` — every scope's own
/// select items, `JOIN ... ON` predicates, and `WHERE`/`HAVING` conjuncts
/// (`crate::analysis::walk::enumerate_select_scopes`), qualifiers
/// discarded. Used to decide which of an upstream's column groups a
/// consumer reads at all (`maintenance::edge_type::type_edge`'s own
/// `consumer_read_columns` parameter — a name-level, not fully-qualified,
/// filter already). Best-effort: a scope the walk cannot normalize
/// contributes no names rather than failing the whole derivation, since
/// this is advisory input to edge typing, not an admission gate.
pub fn referenced_column_names(sql: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(tree) = QueryTree::from_sql(sql) else {
        return names;
    };
    for (sn, _resolver) in crate::analysis::walk::enumerate_select_scopes(&tree) {
        if let Some(items) = select_stmt_items(&sn.select) {
            for item in &items {
                for cref in collect_column_refs(item_expr(item)) {
                    names.insert(cref.name().to_string());
                }
            }
        }
        if let Some(from) = sn.select.from_clause() {
            for join in from.joins() {
                let Some(condition) = join.condition() else {
                    continue;
                };
                let Some(on_expr) = condition.on_expression() else {
                    continue;
                };
                for cref in collect_column_refs(&on_expr) {
                    names.insert(cref.name().to_string());
                }
            }
        }
        let where_expr = sn.select.where_clause().and_then(|w| w.expression());
        let having_expr = sn.select.having_clause().and_then(|h| h.expression());
        for clause_expr in [where_expr, having_expr].into_iter().flatten() {
            for cref in collect_column_refs(&clause_expr) {
                names.insert(cref.name().to_string());
            }
        }
    }
    names
}

/// Derive the output-delta shape per [`ColumnGroup`] (`model_properties.md`
/// §"Output-delta shape": derived per column group, never per model). Runs
/// the composition walk to get a per-output-column verdict, calls the
/// existing [`derive_column_groups`] for the column-group partition
/// (`model_properties.md` §"Per-column mutation-sensitivity / column
/// provenance"), and takes the meet of each group's member columns' shapes.
/// A degenerate/unresolvable group — or a model the walk cannot normalize at
/// all — is `General`, fail-closed. A thin wrapper over
/// [`derive_output_delta_with_model_verdicts`] with no cross-model input —
/// every `models.*` reference falls back to `General` (no behaviour change
/// for existing callers of this function).
pub fn derive_output_delta(
    sql: &str,
    ctx: &JoinContext,
    sources: &[SourceFacts],
    skeleton_columns: &BTreeSet<String>,
) -> Vec<(ColumnGroup, OutputDelta)> {
    derive_output_delta_with_model_verdicts(sql, ctx, sources, skeleton_columns, &BTreeMap::new())
}

/// [`derive_output_delta`]'s full form: `model_verdicts` supplies every
/// referenced model's own already-derived per-output-column facts (the
/// model-reference leaf case), keyed by bare model name — the SAME map
/// [`derive_workspace_output_deltas`] folds across the real workspace.
pub fn derive_output_delta_with_model_verdicts(
    sql: &str,
    ctx: &JoinContext,
    sources: &[SourceFacts],
    skeleton_columns: &BTreeSet<String>,
    model_verdicts: &BTreeMap<String, OutputDeltaFacts>,
) -> Vec<(ColumnGroup, OutputDelta)> {
    let facts = derive_output_delta_facts(sql, ctx, sources, model_verdicts);
    let maintenance_sources = to_maintenance_source_facts(sources);
    let grouping = derive_column_groups(sql, &maintenance_sources, skeleton_columns);

    grouping
        .groups
        .into_iter()
        .map(|group| {
            let shape = group_shape(&group, facts.as_ref());
            (group, shape)
        })
        .collect()
}

/// One model's SQL + facts needed for the cross-model output-delta fold
/// (`derive_workspace_output_deltas`) — mirrors `smelt-runtime`'s own
/// `derive_clamp_and_locality`'s per-model input shape, minimal to what the
/// walk itself reads (no skeleton/grouping inputs — those are per-column
/// facts, not the [`ColumnGroup`] fold).
#[derive(Debug)]
pub struct ModelDeltaInput {
    /// The model's canonical address (`ModelFile::canonical_path()`),
    /// matching the key `models.<address>` references resolve against.
    pub address: String,
    pub sql: String,
    pub ctx: JoinContext,
    pub sources: Vec<SourceFacts>,
}

/// Fold [`OutputDeltaFacts`] across every model reference in the real
/// workspace (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase
/// 4): each pass re-derives every model's facts against the PREVIOUS pass's
/// verdict map, so a chain of `N` model-reference hops converges within `N`
/// passes (mirrors `smelt-runtime::propagation::derive_clamp_and_locality`'s
/// own fixed-point argument). Bounded at `inputs.len() + 1` passes rather
/// than looping to a detected fixed point: a cyclic model-ref graph can
/// never converge to a stable `OutputDelta::General { reason }` (the reason
/// text keeps naming a different intermediate cause each pass), so this
/// terminates fail-closed at `General` for the cyclic/unresolvable members
/// rather than hang (`CLAUDE.md` §"Fail-loud discipline").
pub fn derive_workspace_output_deltas(
    inputs: &[ModelDeltaInput],
) -> BTreeMap<String, OutputDeltaFacts> {
    let max_passes = inputs.len() + 1;
    let mut verdicts: BTreeMap<String, OutputDeltaFacts> = BTreeMap::new();
    for _ in 0..max_passes {
        let mut next = BTreeMap::new();
        for input in inputs {
            let facts =
                derive_output_delta_facts(&input.sql, &input.ctx, &input.sources, &verdicts)
                    .unwrap_or_default();
            next.insert(normalize_model_key(&input.address), facts);
        }
        verdicts = next;
    }
    verdicts
}

fn group_shape(group: &ColumnGroup, facts: Option<&OutputDeltaFacts>) -> OutputDelta {
    let Some(facts) = facts else {
        return OutputDelta::General {
            reason: "model SQL could not be parsed/normalized by the composition walk".to_string(),
        };
    };
    let mut acc: Option<OutputDelta> = None;
    for col in &group.columns {
        let shape = facts
            .columns
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(col))
            .map(|(_, shape)| shape.clone())
            .unwrap_or_else(|| OutputDelta::General {
                reason: format!("column '{col}' has no derived output-delta shape"),
            });
        acc = Some(match acc {
            None => shape,
            Some(a) => a.meet(shape),
        });
    }
    acc.unwrap_or_else(|| OutputDelta::General {
        reason: "column group is empty".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        name: &str,
        profile: Option<MutationProfile>,
        axis: Option<&str>,
        delta_identity: Option<Vec<&str>>,
    ) -> SourceFacts {
        SourceFacts {
            name: name.to_string(),
            axis: axis.map(|a| a.to_string()),
            mutation_profile: profile,
            delta_identity: delta_identity
                .map(|ks| ks.into_iter().map(|k| k.to_string()).collect()),
        }
    }

    fn derive(sql: &str, ctx: &JoinContext, sources: &[SourceFacts]) -> OutputDeltaFacts {
        let tree = QueryTree::from_sql(sql).expect("sql parses to a SELECT");
        let model_verdicts = BTreeMap::new();
        let transfer = OutputDeltaTransfer {
            ctx,
            sources,
            model_verdicts: &model_verdicts,
        };
        walk(&tree, &transfer)
    }

    fn shape_of<'a>(facts: &'a OutputDeltaFacts, col: &str) -> &'a OutputDelta {
        &facts
            .columns
            .iter()
            .find(|(n, _)| n == col)
            .unwrap_or_else(|| panic!("no column '{col}' in {facts:?}"))
            .1
    }

    #[test]
    fn lattice_meet_degrades_never_recovers() {
        let append = OutputDelta::AppendOnlyWindow {
            axis: "event_date".to_string(),
        };
        let keyed = OutputDelta::KeyedUpsert {
            keys: vec!["id".to_string()],
        };
        let general = OutputDelta::General {
            reason: "x".to_string(),
        };

        assert_eq!(append.clone().meet(keyed.clone()), keyed.clone());
        assert_eq!(keyed.clone().meet(append.clone()), keyed);
        assert_eq!(keyed.clone().meet(general.clone()), general.clone());
        assert_eq!(general.clone().meet(keyed), general.clone());
        assert_eq!(append.clone().meet(general.clone()), general.clone());
        assert_eq!(general.clone().meet(append), general);
    }

    #[test]
    fn append_only_source_seeds_window_shape() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let facts = derive(
            "SELECT id, amount FROM smelt.sources.events",
            &ctx,
            &sources,
        );
        assert_eq!(
            *shape_of(&facts, "id"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );
    }

    #[test]
    fn mutable_snapshot_source_seeds_general() {
        let ctx = JoinContext::new();
        let sources = vec![source("dims", Some(MutationProfile::Mutable), None, None)];
        let facts = derive("SELECT id FROM smelt.sources.dims", &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "id"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn change_feed_with_identity_seeds_keyed_upsert() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "cdc",
            Some(MutationProfile::ChangeFeed),
            None,
            Some(vec!["commit_version", "row_offset"]),
        )];
        let facts = derive("SELECT id FROM smelt.sources.cdc", &ctx, &sources);
        assert_eq!(
            *shape_of(&facts, "id"),
            OutputDelta::KeyedUpsert {
                keys: vec!["commit_version".to_string(), "row_offset".to_string()]
            }
        );
    }

    #[test]
    fn undeclared_profile_seeds_general_naming_the_source() {
        let ctx = JoinContext::new();
        let sources = vec![source("mystery", None, None, None)];
        let facts = derive("SELECT id FROM smelt.sources.mystery", &ctx, &sources);
        match shape_of(&facts, "id") {
            OutputDelta::General { reason } => assert!(reason.contains("mystery")),
            other => panic!("expected General naming the source, got {other:?}"),
        }
    }

    #[test]
    fn filter_preserves_input_shape() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let facts = derive(
            "SELECT id, amount FROM smelt.sources.events WHERE amount > 0",
            &ctx,
            &sources,
        );
        assert_eq!(
            *shape_of(&facts, "amount"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );
    }

    #[test]
    fn projection_preserves_input_shape() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let facts = derive(
            "SELECT id, amount * 2 AS doubled FROM smelt.sources.events",
            &ctx,
            &sources,
        );
        assert_eq!(
            *shape_of(&facts, "doubled"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );
    }

    #[test]
    fn union_all_takes_the_meet_of_arms() {
        let ctx = JoinContext::new();
        let sources = vec![
            source("a", Some(MutationProfile::AppendOnly), Some("dt"), None),
            source("b", Some(MutationProfile::Mutable), None, None),
        ];
        let sql = "SELECT id FROM smelt.sources.a UNION ALL SELECT id FROM smelt.sources.b";
        let facts = derive(sql, &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "id"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn group_by_over_append_only_emits_keyed_upsert() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let sql = "SELECT user_id, COUNT(*) AS n FROM smelt.sources.events GROUP BY user_id";
        let facts = derive(sql, &ctx, &sources);
        assert_eq!(
            *shape_of(&facts, "user_id"),
            OutputDelta::KeyedUpsert {
                keys: vec!["user_id".to_string()]
            }
        );
    }

    #[test]
    fn group_by_over_general_stays_general() {
        let ctx = JoinContext::new();
        let sources = vec![source("dims", Some(MutationProfile::Mutable), None, None)];
        let sql = "SELECT user_id, COUNT(*) AS n FROM smelt.sources.dims GROUP BY user_id";
        let facts = derive(sql, &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "user_id"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn join_takes_the_meet() {
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let sources = vec![
            source(
                "events",
                Some(MutationProfile::AppendOnly),
                Some("event_date"),
                None,
            ),
            source("dims", Some(MutationProfile::Mutable), None, None),
        ];
        let sql = "SELECT e.id AS eid, d.id AS did, e.amount + d.weight AS combined \
                   FROM smelt.sources.events e JOIN smelt.sources.dims d ON e.dim_id = d.id";
        let facts = derive(sql, &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "eid"),
            OutputDelta::AppendOnlyWindow { .. }
        ));
        assert!(matches!(
            shape_of(&facts, "did"),
            OutputDelta::General { .. }
        ));
        assert!(matches!(
            shape_of(&facts, "combined"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn one_to_many_join_degrades_to_general() {
        // No declared unique key: fan_out fails closed to OneToMany.
        let ctx = JoinContext::new();
        let sources = vec![
            source(
                "events",
                Some(MutationProfile::AppendOnly),
                Some("event_date"),
                None,
            ),
            source(
                "dims",
                Some(MutationProfile::AppendOnly),
                Some("event_date"),
                None,
            ),
        ];
        let sql = "SELECT e.id AS eid \
                   FROM smelt.sources.events e JOIN smelt.sources.dims d ON e.dim_id = d.id";
        let facts = derive(sql, &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "eid"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn window_function_column_is_general() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let sql =
            "SELECT id, SUM(amount) OVER (PARTITION BY user_id ORDER BY event_date) AS running \
                   FROM smelt.sources.events";
        let facts = derive(sql, &ctx, &sources);
        assert!(matches!(
            shape_of(&facts, "running"),
            OutputDelta::General { .. }
        ));
        // Sibling non-window column in the same scope keeps its own shape —
        // no whole-scope collapse.
        assert_eq!(
            *shape_of(&facts, "id"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );
    }

    #[test]
    fn unregistered_operator_is_general_naming_the_operator() {
        let ctx = JoinContext::new();
        let sources = vec![
            source("a", Some(MutationProfile::AppendOnly), Some("dt"), None),
            source("b", Some(MutationProfile::AppendOnly), Some("dt"), None),
        ];
        let sql = "SELECT id FROM smelt.sources.a INTERSECT SELECT id FROM smelt.sources.b";
        let facts = derive(sql, &ctx, &sources);
        match shape_of(&facts, "id") {
            OutputDelta::General { reason } => {
                assert!(reason.to_uppercase().contains("INTERSECT"))
            }
            other => panic!("expected General naming the operator, got {other:?}"),
        }
    }

    #[test]
    fn cte_and_derived_table_compose_through_the_walk() {
        let ctx = JoinContext::new();
        let sources = vec![source(
            "events",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        )];
        let sql = "WITH base AS (SELECT id, amount FROM smelt.sources.events) \
                   SELECT renamed_id FROM (SELECT id AS renamed_id FROM base) sub";
        let facts = derive(sql, &ctx, &sources);
        assert_eq!(
            *shape_of(&facts, "renamed_id"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );
    }

    fn source_info(
        has_timeseries: bool,
        mutation_profile: Option<smelt_core::sources::MutationProfile>,
        delta_identity: Option<Vec<&str>>,
    ) -> smelt_core::sources::SourceInfo {
        smelt_core::sources::SourceInfo {
            path: std::path::PathBuf::from("/tmp/fake.yml"),
            address_segments: vec!["fake".to_string()],
            columns: vec![],
            description: None,
            name_override: None,
            tags: vec![],
            timeseries: has_timeseries.then(|| smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            mutation_profile: mutation_profile.map(|kind| {
                let mut profile = smelt_core::sources::SourceMutationProfile::from_kind(kind);
                profile.delta_identity =
                    delta_identity.map(|ks| ks.into_iter().map(|k| k.to_string()).collect());
                profile
            }),
            source_lateness: None,
            watermark: None,
            unique_key: None,
            retention: None,
            referential_integrity: None,
        }
    }

    #[test]
    fn source_facts_from_source_info_seeds_leaf() {
        let append_only = source_info(
            true,
            Some(smelt_core::sources::MutationProfile::AppendOnly),
            None,
        );
        let facts = SourceFacts::from_source_info("events", &append_only);
        assert_eq!(facts.name, "events");
        assert_eq!(facts.axis.as_deref(), Some("event_date"));
        assert_eq!(
            OutputDeltaTransfer {
                ctx: &JoinContext::new(),
                sources: &[facts],
                model_verdicts: &BTreeMap::new(),
            }
            .seed_for_source_name("events"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );

        let change_feed = source_info(
            false,
            Some(smelt_core::sources::MutationProfile::ChangeFeed),
            Some(vec!["commit_version"]),
        );
        let facts = SourceFacts::from_source_info("cdc", &change_feed);
        assert_eq!(
            OutputDeltaTransfer {
                ctx: &JoinContext::new(),
                sources: &[facts],
                model_verdicts: &BTreeMap::new(),
            }
            .seed_for_source_name("cdc"),
            OutputDelta::KeyedUpsert {
                keys: vec!["commit_version".to_string()]
            }
        );

        let undeclared = source_info(false, None, None);
        let facts = SourceFacts::from_source_info("mystery", &undeclared);
        assert!(matches!(
            OutputDeltaTransfer {
                ctx: &JoinContext::new(),
                sources: &[facts],
                model_verdicts: &BTreeMap::new(),
            }
            .seed_for_source_name("mystery"),
            OutputDelta::General { .. }
        ));
    }

    #[test]
    fn groups_are_independent() {
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let sources = vec![
            source(
                "events",
                Some(MutationProfile::AppendOnly),
                Some("event_date"),
                None,
            ),
            source("dims", Some(MutationProfile::Mutable), None, None),
        ];
        let sql = "SELECT e.amount AS amt, d.weight AS wt \
                   FROM smelt.sources.events e JOIN smelt.sources.dims d ON e.dim_id = d.id";
        let skeleton = BTreeSet::new();
        let groups = derive_output_delta(sql, &ctx, &sources, &skeleton);

        assert_eq!(
            groups.len(),
            2,
            "expected two independent groups, got {groups:?}"
        );
        let amt_group = groups
            .iter()
            .find(|(g, _)| g.columns.contains(&"amt".to_string()))
            .expect("an 'amt' group exists");
        let wt_group = groups
            .iter()
            .find(|(g, _)| g.columns.contains(&"wt".to_string()))
            .expect("a 'wt' group exists");
        assert!(matches!(amt_group.1, OutputDelta::AppendOnlyWindow { .. }));
        assert!(matches!(wt_group.1, OutputDelta::General { .. }));
    }

    #[test]
    fn model_reference_leaf_resolves_per_column_from_upstream_facts() {
        let ctx = JoinContext::new();
        let mut model_verdicts = BTreeMap::new();
        model_verdicts.insert(
            "upstream".to_string(),
            OutputDeltaFacts {
                columns: vec![
                    (
                        "a".to_string(),
                        OutputDelta::AppendOnlyWindow {
                            axis: "event_date".to_string(),
                        },
                    ),
                    (
                        "b".to_string(),
                        OutputDelta::KeyedUpsert {
                            keys: vec!["id".to_string()],
                        },
                    ),
                ],
            },
        );
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &model_verdicts,
        };
        let tree = QueryTree::from_sql("SELECT a, b FROM smelt.models.upstream")
            .expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        assert_eq!(
            *shape_of(&facts, "a"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            },
            "column 'a' must keep its own upstream shape, not be meet-folded with 'b'"
        );
        assert_eq!(
            *shape_of(&facts, "b"),
            OutputDelta::KeyedUpsert {
                keys: vec!["id".to_string()]
            }
        );
    }

    #[test]
    fn model_reference_column_absent_from_upstream_is_general() {
        let ctx = JoinContext::new();
        let mut model_verdicts = BTreeMap::new();
        model_verdicts.insert(
            "upstream".to_string(),
            OutputDeltaFacts {
                columns: vec![(
                    "a".to_string(),
                    OutputDelta::AppendOnlyWindow {
                        axis: "event_date".to_string(),
                    },
                )],
            },
        );
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &model_verdicts,
        };
        let tree = QueryTree::from_sql("SELECT missing_col FROM smelt.models.upstream")
            .expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        match shape_of(&facts, "missing_col") {
            OutputDelta::General { reason } => {
                assert!(reason.contains("upstream"));
                assert!(reason.contains("missing_col"));
            }
            other => panic!("expected General naming model + column, got {other:?}"),
        }
    }

    #[test]
    fn bare_model_reference_leaf_resolves_through_model_verdicts() {
        let ctx = JoinContext::new();
        let mut model_verdicts = BTreeMap::new();
        model_verdicts.insert(
            "upstream".to_string(),
            OutputDeltaFacts {
                columns: vec![
                    (
                        "id".to_string(),
                        OutputDelta::KeyedUpsert {
                            keys: vec!["id".to_string()],
                        },
                    ),
                    (
                        "amount".to_string(),
                        OutputDelta::AppendOnlyWindow {
                            axis: "event_date".to_string(),
                        },
                    ),
                ],
            },
        );
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &model_verdicts,
        };
        let tree = QueryTree::from_sql("SELECT id, amount FROM smelt.upstream")
            .expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        assert_eq!(
            *shape_of(&facts, "id"),
            OutputDelta::KeyedUpsert {
                keys: vec!["id".to_string()]
            },
            "bare ref must resolve the per-column path, not fail closed"
        );
        assert_eq!(
            *shape_of(&facts, "amount"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            }
        );

        // Whole-leaf path (`SELECT *`-shaped leaf seeding) resolves the same way.
        let whole_leaf = transfer.seed_for_leaf_name("upstream");
        assert_ne!(
            whole_leaf,
            OutputDelta::General {
                reason: "unused".to_string()
            },
            "whole-leaf resolution must not be a General placeholder"
        );
        assert!(
            !matches!(whole_leaf, OutputDelta::General { .. }),
            "bare whole-leaf reference must resolve through model_verdicts, got {whole_leaf:?}"
        );
    }

    #[test]
    fn model_key_lookup_is_breadcrumb_insensitive() {
        let ctx = JoinContext::new();

        // smelt-db key form (`models.<addr>`) resolves for a bare SQL spelling.
        let mut breadcrumbed = BTreeMap::new();
        breadcrumbed.insert(
            "models.upstream".to_string(),
            OutputDeltaFacts {
                columns: vec![(
                    "a".to_string(),
                    OutputDelta::KeyedUpsert {
                        keys: vec!["id".to_string()],
                    },
                )],
            },
        );
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &breadcrumbed,
        };
        let tree =
            QueryTree::from_sql("SELECT a FROM smelt.upstream").expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        assert_eq!(
            *shape_of(&facts, "a"),
            OutputDelta::KeyedUpsert {
                keys: vec!["id".to_string()]
            }
        );

        // Runtime key form (bare `<addr>`) resolves for a breadcrumbed SQL spelling.
        let mut bare = BTreeMap::new();
        bare.insert(
            "upstream".to_string(),
            OutputDeltaFacts {
                columns: vec![(
                    "a".to_string(),
                    OutputDelta::KeyedUpsert {
                        keys: vec!["id".to_string()],
                    },
                )],
            },
        );
        let transfer2 = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &bare,
        };
        let tree2 = QueryTree::from_sql("SELECT a FROM smelt.models.upstream")
            .expect("sql parses to a SELECT");
        let facts2 = walk(&tree2, &transfer2);
        assert_eq!(
            *shape_of(&facts2, "a"),
            OutputDelta::KeyedUpsert {
                keys: vec!["id".to_string()]
            }
        );
    }

    #[test]
    fn declared_source_wins_over_same_named_model_for_bare_ref() {
        let ctx = JoinContext::new();
        let src = source(
            "shared_name",
            Some(MutationProfile::AppendOnly),
            Some("event_date"),
            None,
        );
        let mut model_verdicts = BTreeMap::new();
        model_verdicts.insert(
            "shared_name".to_string(),
            OutputDeltaFacts {
                columns: vec![(
                    "a".to_string(),
                    OutputDelta::KeyedUpsert {
                        keys: vec!["id".to_string()],
                    },
                )],
            },
        );
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[src],
            model_verdicts: &model_verdicts,
        };
        let tree =
            QueryTree::from_sql("SELECT a FROM smelt.shared_name").expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        assert_eq!(
            *shape_of(&facts, "a"),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string()
            },
            "a declared source must win over a same-named model verdict for a bare ref"
        );
    }

    #[test]
    fn bare_ref_matching_neither_names_both_misses() {
        let ctx = JoinContext::new();
        let model_verdicts = BTreeMap::new();
        let transfer = OutputDeltaTransfer {
            ctx: &ctx,
            sources: &[],
            model_verdicts: &model_verdicts,
        };
        let tree =
            QueryTree::from_sql("SELECT a FROM smelt.nowhere").expect("sql parses to a SELECT");
        let facts = walk(&tree, &transfer);
        match shape_of(&facts, "a") {
            OutputDelta::General { reason } => {
                assert!(reason.contains("nowhere"), "reason: {reason}");
                assert!(
                    reason.contains("mutation profile") && reason.contains("model"),
                    "reason must name both misses: {reason}"
                );
            }
            other => panic!("expected General naming both misses, got {other:?}"),
        }
    }
}
