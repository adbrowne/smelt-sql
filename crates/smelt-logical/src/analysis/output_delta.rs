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
#[derive(Debug, Clone, Default)]
pub struct OutputDeltaFacts {
    pub columns: Vec<(String, OutputDelta)>,
}

/// The output-delta transfer function (`docs/specs/model_properties.md`
/// §"Output-delta shape" transfer-rule table), run over the shared
/// composition walk (`crate::analysis::walk`).
pub struct OutputDeltaTransfer<'a> {
    pub ctx: &'a JoinContext,
    pub sources: &'a [SourceFacts],
    /// A referenced model's own already-derived output-delta verdict, keyed
    /// by bare model name (case-insensitive) — the model-reference leaf case
    /// (`model_properties.md` §"Output-delta shape"). No consumer wires this
    /// yet (a future cross-model fold reads it); an absent entry falls back
    /// to `General`, never an optimistic guess.
    pub model_verdicts: &'a BTreeMap<String, OutputDelta>,
}

impl OutputDeltaTransfer<'_> {
    /// Leaf-seeding rule (`model_properties.md` §"Output-delta shape",
    /// base-relation row): `append_only` with a declared clock ⇒
    /// `AppendOnlyWindow`; `change_feed` with a `delta_identity` ⇒
    /// `KeyedUpsert`; everything else ⇒ `General`, fail-closed — mirrors
    /// `input_delta::input_delta_discovery`'s fail-closed default.
    fn seed_for_source(&self, facts: &SourceFacts) -> OutputDelta {
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

    /// Resolve a leaf relation's name (as normalized by the walk:
    /// `sources.<name>`, `models.<name>`, or a bare table name) to its
    /// seeded [`OutputDelta`].
    fn seed_for_leaf_name(&self, name: &str) -> OutputDelta {
        if let Some(bare) = name.strip_prefix("sources.") {
            return self.seed_for_source_name(bare);
        }
        if let Some(bare) = name.strip_prefix("models.") {
            return self
                .model_verdicts
                .get(&bare.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| OutputDelta::General {
                    reason: format!(
                        "referenced model '{bare}' has no derived output-delta verdict available"
                    ),
                });
        }
        self.seed_for_source_name(name)
    }

    fn seed_for_source_name(&self, bare: &str) -> OutputDelta {
        match self
            .sources
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(bare))
        {
            Some(facts) => self.seed_for_source(facts),
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
            Some(RelationSource::Table(name)) => self.seed_for_leaf_name(name),
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

/// Derive the output-delta shape per [`ColumnGroup`] (`model_properties.md`
/// §"Output-delta shape": derived per column group, never per model). Runs
/// the composition walk to get a per-output-column verdict, calls the
/// existing [`derive_column_groups`] for the column-group partition
/// (`model_properties.md` §"Per-column mutation-sensitivity / column
/// provenance"), and takes the meet of each group's member columns' shapes.
/// A degenerate/unresolvable group — or a model the walk cannot normalize at
/// all — is `General`, fail-closed.
pub fn derive_output_delta(
    sql: &str,
    ctx: &JoinContext,
    sources: &[SourceFacts],
    skeleton_columns: &BTreeSet<String>,
) -> Vec<(ColumnGroup, OutputDelta)> {
    let empty_model_verdicts = BTreeMap::new();
    let facts = QueryTree::from_sql(sql).map(|tree| {
        let transfer = OutputDeltaTransfer {
            ctx,
            sources,
            model_verdicts: &empty_model_verdicts,
        };
        walk(&tree, &transfer)
    });

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
}
