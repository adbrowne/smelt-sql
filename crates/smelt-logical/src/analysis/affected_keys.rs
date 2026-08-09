//! Affected-key discovery proof (`model_properties.md` §"Affected-key
//! discovery"): from a changed input's delta rows, the finite set of output
//! group keys the repair family (`incremental_models.md` §"The repair
//! family") must recompute. This is admission obligation 7 of the repair
//! family's per-cell admission (`incremental_models.md` §"Per-cell
//! admission"): a proof only, consumed by plan derivation but not yet
//! wired into it (that lands with the per-group recompute technique).
//!
//! **Walk-composed, not a whole-text scan.** The grain — the same
//! `GROUP BY`/key-derivation the walk already proves for region row
//! identity (`maintenance::derive::row_identity`) and column-group
//! factoring — is resolved via `analysis::walk::model_property_vector`,
//! fan-out-gated exactly as `row_identity_with_context` gates its own
//! proven-key read. Each grain column's source-column dependency set is
//! then resolved through the shared walk lineage, reusing
//! `analysis::fingerprint`'s per-column leaf classifier
//! (`classify_ref`/`scan_expr_for_source`) parameterised with a
//! target-output-column filter, rather than a second copy of that
//! classifier.
//!
//! **Fail-closed** (`model_properties.md` §"Affected-key discovery"): an
//! unkeyed retraction (no row identity to project through the grain
//! expression), a grain expression reading a column absent from the
//! delta's own row shape, an unresolvable/opaque grain-column expression, a
//! top-level set-operation root, or a model with no proven grain and no
//! declared unique key all yield [`AffectedKeys::NotDiscoverable`] naming
//! the reason, never a guessed key set. A **sound over-approximation** — a
//! superset of the true affected keys — is admissible; an
//! under-approximation never is.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::fingerprint::{relation_matches_source, scan_expr_for_source, ExprTouch};
use super::join_shape::JoinContext;
use super::walk::{
    model_property_vector, walk, LeafInput, NodeCx, OpNode, QueryNode, QueryTree, Transfer,
};

/// The P7 verdict: either the finite set of output grain columns a changed
/// input's delta can affect, or a fail-closed refusal naming why the delta
/// shape could not be resolved to a key set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AffectedKeys {
    /// The model's grain columns — a sound over-approximation of the truly
    /// affected keys is admissible.
    Keys { cols: Vec<String> },
    /// Fail-closed: the delta shape cannot be resolved to a key set.
    NotDiscoverable { reason: String },
}

/// The row shape of a changed input's delta: which source produced it,
/// which columns its rows carry, and whether its rows carry a per-row
/// identity at all (an unkeyed retraction carries none).
#[derive(Debug, Clone)]
pub struct DeltaShape {
    /// The source (or alias) the delta's columns are read against — the
    /// same bare/`sources.`-prefixed convention
    /// [`super::fingerprint::fingerprint_projection`]'s `source_ref` uses.
    pub source: String,
    /// The column names the delta's own rows carry.
    pub columns: BTreeSet<String>,
    /// Whether the delta carries a per-row identity to project through the
    /// grain expression. `false` (an unkeyed retraction) always fails
    /// closed regardless of `columns`.
    pub keyed: bool,
}

/// Context folded into the grain derivation: a declared `unique_key`
/// (wins the same precedence `maintenance::derive::row_identity_with_context`
/// uses) and the [`JoinContext`] the walk's fan-out gate needs.
#[derive(Debug, Default)]
pub struct AffectedKeyContext {
    pub unique_key: Vec<String>,
    pub join: JoinContext,
}

/// Derive the affected-key set (P7) for `delta` against `model_sql`.
pub fn derive_affected_keys(
    delta: &DeltaShape,
    model_sql: &str,
    ctx: &AffectedKeyContext,
) -> AffectedKeys {
    if !delta.keyed {
        return AffectedKeys::NotDiscoverable {
            reason: "delta carries no per-row identity (unkeyed retraction) to project through \
                     the grain expression"
                .to_string(),
        };
    }

    let Some(tree) = QueryTree::from_sql(model_sql) else {
        return AffectedKeys::NotDiscoverable {
            reason: "model SQL has no top-level SELECT to classify".to_string(),
        };
    };
    if tree.root.has_unsupported() {
        return AffectedKeys::NotDiscoverable {
            reason: "model SQL contains a construct the composition walk cannot normalize"
                .to_string(),
        };
    }
    if matches!(tree.root, QueryNode::SetOp(_)) {
        return AffectedKeys::NotDiscoverable {
            reason: "model SQL's outermost query is a set operation — affected-key discovery is \
                     not yet classified across set-operation arms"
                .to_string(),
        };
    }

    let Some(grain_cols) = resolve_grain(model_sql, ctx) else {
        return AffectedKeys::NotDiscoverable {
            reason: "model has no proven grain and no declared unique key".to_string(),
        };
    };

    let targets: BTreeSet<String> = grain_cols.iter().map(|c| c.to_ascii_lowercase()).collect();
    let transfer = GrainProvenanceTransfer {
        source_ref: &delta.source,
        targets: &targets,
    };
    let resolved = walk(&tree, &transfer);

    let delta_columns: BTreeSet<String> = delta
        .columns
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect();

    let mut required = BTreeSet::new();
    for col in &grain_cols {
        match resolved.get(&col.to_ascii_lowercase()) {
            Some(Ok(cols)) => required.extend(cols.iter().cloned()),
            Some(Err(reason)) => {
                return AffectedKeys::NotDiscoverable {
                    reason: reason.clone(),
                };
            }
            None => {
                return AffectedKeys::NotDiscoverable {
                    reason: format!(
                        "grain column '{col}' could not be resolved against the composition walk"
                    ),
                };
            }
        }
    }

    for col in &required {
        if !delta_columns.contains(col) {
            return AffectedKeys::NotDiscoverable {
                reason: format!(
                    "grain expression reads column '{col}' absent from the delta's own row shape"
                ),
            };
        }
    }

    AffectedKeys::Keys { cols: grain_cols }
}

/// The grain columns to project the delta through: a declared `unique_key`
/// first, else the walk's own proven grain key — fan-out-gated exactly as
/// `maintenance::derive::row_identity_with_context` gates its proven-key
/// read. `None` when neither is available.
fn resolve_grain(model_sql: &str, ctx: &AffectedKeyContext) -> Option<Vec<String>> {
    if !ctx.unique_key.is_empty() {
        return Some(ctx.unique_key.clone());
    }
    let vector = model_property_vector(model_sql, &ctx.join)?;
    if vector.has_fan_out_join {
        return None;
    }
    vector.grain.keys.into_iter().next()
}

/// One grain column's resolution: the `source_ref` columns it depends on
/// (possibly empty when the column does not touch `source_ref` at all), or
/// a fail-closed reason.
type ColumnResolution = Result<BTreeSet<String>, String>;

/// Walks the tree solely to resolve the root scope's grain columns against
/// `source_ref`; nested CTE/derived-table scopes are also classified (the
/// walk contract requires every scope produce a verdict) but only the root
/// scope's result is read by [`derive_affected_keys`] — the same discarded-
/// intermediate-work pattern `fingerprint::FingerprintTransfer` uses.
struct GrainProvenanceTransfer<'a> {
    source_ref: &'a str,
    targets: &'a BTreeSet<String>,
}

impl Transfer for GrainProvenanceTransfer<'_> {
    type Verdict = BTreeMap<String, ColumnResolution>;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Self::Verdict {
        BTreeMap::new()
    }

    fn operator(&self, op: &OpNode<'_>, _children: &[Self::Verdict], cx: &NodeCx) -> Self::Verdict {
        match op {
            OpNode::Select(sn) => {
                resolve_target_columns(&sn.select, cx, self.source_ref, self.targets)
            }
            OpNode::SetOp(_) | OpNode::Unsupported { .. } => BTreeMap::new(),
        }
    }
}

/// Resolve just the select items whose output name (lower-cased) is in
/// `targets`, reusing the walk-resolved rename-chain lineage
/// (`cx.columns[idx].leaf`) when available and falling back to
/// `scan_expr_for_source`'s leaf classifier for a computed expression —
/// the same two-step precedent `fingerprint::select_projection` uses,
/// restricted here to the target columns instead of the whole select list.
fn resolve_target_columns(
    select: &smelt_parser::SelectStmt,
    cx: &NodeCx,
    source_ref: &str,
    targets: &BTreeSet<String>,
) -> BTreeMap<String, ColumnResolution> {
    let mut out = BTreeMap::new();
    let Some(select_list) = select.select_list() else {
        return out;
    };
    let mut idx = 0usize;
    for item in select_list.items() {
        if item.is_wildcard() {
            idx += 1;
            continue;
        }
        let Some(expr) = item.expression() else {
            idx += 1;
            continue;
        };
        let lineage = cx.columns.get(idx).cloned();
        idx += 1;

        let Some(lineage) = lineage else { continue };
        let output_key = lineage.output.to_ascii_lowercase();
        if !targets.contains(&output_key) {
            continue;
        }

        if let Some(leaf) = &lineage.leaf {
            if relation_matches_source(&leaf.relation, source_ref) {
                out.insert(output_key, Ok([leaf.column.clone()].into_iter().collect()));
            } else {
                out.insert(output_key, Ok(BTreeSet::new()));
            }
            continue;
        }

        let resolution = match scan_expr_for_source(&expr, cx, source_ref) {
            ExprTouch::None => Ok(BTreeSet::new()),
            ExprTouch::Columns(cols) => Ok(cols),
            ExprTouch::Refuse(reason) => Err(reason),
        };
        out.insert(output_key, resolution);
    }
    out
}

/// `AffectedKeyContext` construction from a declared `unique_key` alone —
/// convenience sugar mirroring `JoinContext::new()`'s always-empty default.
impl AffectedKeyContext {
    pub fn with_unique_key(columns: Vec<String>) -> Self {
        AffectedKeyContext {
            unique_key: columns,
            join: JoinContext::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed_delta(source: &str, columns: &[&str]) -> DeltaShape {
        DeltaShape {
            source: source.to_string(),
            columns: columns.iter().map(|c| c.to_string()).collect(),
            keyed: true,
        }
    }

    #[test]
    fn unkeyed_retraction_is_not_discoverable() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM smelt.sources.orders \
                    GROUP BY customer_id";
        let delta = DeltaShape {
            source: "orders".to_string(),
            columns: ["customer_id".to_string()].into_iter().collect(),
            keyed: false,
        };
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        match verdict {
            AffectedKeys::NotDiscoverable { reason } => {
                assert!(reason.contains("unkeyed"), "{reason}");
            }
            other => panic!("expected NotDiscoverable, got {other:?}"),
        }
    }

    #[test]
    fn group_by_key_over_present_delta_columns_yields_keys() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM smelt.sources.orders \
                    GROUP BY customer_id";
        let delta = keyed_delta("orders", &["customer_id", "amount"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        assert_eq!(
            verdict,
            AffectedKeys::Keys {
                cols: vec!["customer_id".to_string()]
            }
        );
    }

    #[test]
    fn grain_expression_reading_absent_delta_column_is_not_discoverable() {
        let sql = "SELECT date_trunc('day', event_ts) AS day, COUNT(*) AS n \
                    FROM smelt.sources.events GROUP BY date_trunc('day', event_ts)";
        let delta = keyed_delta("events", &["user_id"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        match verdict {
            AffectedKeys::NotDiscoverable { reason } => {
                assert!(reason.contains("event_ts"), "{reason}");
            }
            other => panic!("expected NotDiscoverable, got {other:?}"),
        }
    }

    #[test]
    fn grain_expression_over_present_delta_column_is_discoverable() {
        let sql = "SELECT date_trunc('day', event_ts) AS day, COUNT(*) AS n \
                    FROM smelt.sources.events GROUP BY date_trunc('day', event_ts)";
        let delta = keyed_delta("events", &["event_ts"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        assert_eq!(
            verdict,
            AffectedKeys::Keys {
                cols: vec!["day".to_string()]
            }
        );
    }

    #[test]
    fn model_with_no_proven_grain_is_not_discoverable() {
        let sql = "SELECT customer_id, amount FROM smelt.sources.orders";
        let delta = keyed_delta("orders", &["customer_id", "amount"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        match verdict {
            AffectedKeys::NotDiscoverable { reason } => {
                assert!(reason.contains("no proven grain"), "{reason}");
            }
            other => panic!("expected NotDiscoverable, got {other:?}"),
        }
    }

    #[test]
    fn declared_unique_key_supplies_the_grain_when_walk_proves_none() {
        let sql = "SELECT customer_id, amount FROM smelt.sources.orders";
        let delta = keyed_delta("orders", &["customer_id", "amount"]);
        let ctx = AffectedKeyContext::with_unique_key(vec!["customer_id".to_string()]);
        let verdict = derive_affected_keys(&delta, sql, &ctx);
        assert_eq!(
            verdict,
            AffectedKeys::Keys {
                cols: vec!["customer_id".to_string()]
            }
        );
    }

    #[test]
    fn delta_carrying_extra_columns_is_still_discoverable() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM smelt.sources.orders \
                    GROUP BY customer_id";
        let delta = keyed_delta("orders", &["customer_id", "amount", "region", "created_at"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        assert_eq!(
            verdict,
            AffectedKeys::Keys {
                cols: vec!["customer_id".to_string()]
            }
        );
    }

    #[test]
    fn set_operation_root_is_not_discoverable() {
        let sql = "SELECT customer_id FROM smelt.sources.orders \
                    UNION ALL \
                    SELECT customer_id FROM smelt.sources.returns";
        let delta = keyed_delta("orders", &["customer_id"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        match verdict {
            AffectedKeys::NotDiscoverable { reason } => {
                assert!(reason.contains("set operation"), "{reason}");
            }
            other => panic!("expected NotDiscoverable, got {other:?}"),
        }
    }

    #[test]
    fn opaque_expression_provenance_is_not_discoverable() {
        let sql = "SELECT custom_udf(customer_id) AS key, SUM(amount) AS total \
                    FROM smelt.sources.orders GROUP BY custom_udf(customer_id)";
        let delta = keyed_delta("orders", &["customer_id", "amount"]);
        let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
        match verdict {
            AffectedKeys::NotDiscoverable { reason } => {
                assert!(reason.contains("opaque function"), "{reason}");
            }
            other => panic!("expected NotDiscoverable, got {other:?}"),
        }
    }
}
