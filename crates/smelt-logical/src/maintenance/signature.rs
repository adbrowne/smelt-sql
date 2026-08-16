//! The delta-signature headline (`docs/specs/incremental_models.md` §Surface
//! "CLI" — `smelt explain <model>` **Headline**): a pure formatter over a
//! model's own derived per-column-group [`OutputDelta`] verdicts, its
//! resolved grain label, its derived run shape, and (for a `grain: key`
//! model) its admitted key-temporal-locality slice bound. Single-owned here
//! so `smelt explain`'s text and `--json` renderings can never drift — the
//! CLI formats nothing of its own, it only reads this struct's fields.
//!
//! **Widen-never-narrow.** The headline's own addressing derivation reuses
//! [`crate::maintenance::edge_type::Addressing`]'s three-way mapping
//! (`AppendOnlyWindow` → `Window`, `KeyedUpsert` → `Keyed`, `General` →
//! `WholeModel`) — the same rule an edge's own typed component uses, applied
//! here to the model's own emitted shape rather than a projection through a
//! consumer. A model with no derivable verdict at all prints no `emits:`
//! clause rather than a fabricated `general` ([`derive_signature_headline`]
//! returns `None`).

use crate::analysis::output_delta::OutputDelta;
use crate::maintenance::edge_type::Addressing;

/// The model's derived run shape (`incremental_shapes.md` §"The two run
/// shapes (derived, never declared)" for `grain: key`; the partition-grain
/// window sweep for `grain: partition`). Named `Keyed` for its `grain: key`
/// origin even though [`PartitionSweep`](Self::PartitionSweep) also derives
/// from this type — a single run-shape vocabulary, not two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyedRunShape {
    /// A clocked `grain: key` model: the driving source carries a
    /// `timeseries:` clock, so the run advances window-forward.
    WindowForward,
    /// An unclocked `grain: key` model: no driving-source clock, so every
    /// run reconciles the whole keyed end-state
    /// (`CumulativeClassification::is_snapshot_reconcile`).
    SnapshotReconcile,
    /// A `grain: partition` model: the run sweeps the declared partition
    /// axis.
    PartitionSweep { axis: String },
}

impl KeyedRunShape {
    /// Short label for the headline's `run shape:` clause.
    pub fn label(&self) -> String {
        match self {
            KeyedRunShape::WindowForward => "window-forward".to_string(),
            KeyedRunShape::SnapshotReconcile => "snapshot-reconcile".to_string(),
            KeyedRunShape::PartitionSweep { axis } => format!("window sweep over {axis}"),
        }
    }
}

/// The rendered delta-signature headline
/// (`docs/specs/incremental_models.md` §Surface "CLI" **Headline**): the
/// model's own emitted [`OutputDelta`] shape (the meet across whatever
/// per-column-group verdicts [`derive_signature_headline`] was given),
/// its projected [`Addressing`], the resolved grain label, the optional
/// derived run shape, and the optional key-temporal-locality slice bound.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureHeadline {
    /// The model's own emitted shape — the widest (meet) verdict across the
    /// per-group verdicts this headline was derived from.
    pub emits: OutputDelta,
    /// `emits` projected to an addressing, via the same rule
    /// [`crate::maintenance::edge_type::type_edge`] uses.
    pub addressing: Addressing,
    /// The column group that forced `emits` to `General`, when it is one —
    /// `None` for a `Keyed`/`Window` emits, or a single-group `General`
    /// verdict with nothing to distinguish it from (still named, since a
    /// single group IS the degrading one).
    pub degrading_group: Option<String>,
    /// The model's resolved grain label (`RelationContractView::
    /// derived_grain`'s own text — never re-derived here).
    pub grain_label: String,
    pub run_shape: Option<KeyedRunShape>,
    /// The admitted key-temporal-locality route's short label
    /// (`KeyLocality::slice`'s own route classification — never re-derived
    /// here), for a `grain: key` model that established one.
    pub locality_bound: Option<String>,
}

/// Derive the headline from `verdicts` — a model's own per-column-group
/// output-delta verdicts, e.g. the ones
/// `smelt_db::own_output_delta_verdicts` (or the equivalent per-workspace
/// fold `ref_model_edge`'s `output_shape` reduces via [`OutputDelta::meet`])
/// derives for the model itself. Returns `None` for an empty `verdicts` —
/// no derivable verdict, never a fabricated `general` headline.
///
/// The winning verdict is the highest-[`OutputDelta::rank`] (widest) group —
/// the same degrade-only meet [`OutputDelta::meet`] performs, but tracked
/// here alongside the winning group's own name so a `General` verdict born
/// from a mix of narrower siblings can name which group forced the degrade.
pub fn derive_signature_headline(
    verdicts: &[(String, OutputDelta)],
    grain_label: String,
    run_shape: Option<KeyedRunShape>,
    locality_bound: Option<String>,
) -> Option<SignatureHeadline> {
    let mut winner: Option<&(String, OutputDelta)> = None;
    for gv in verdicts {
        winner = match winner {
            Some(w) if w.1.rank() >= gv.1.rank() => Some(w),
            _ => Some(gv),
        };
    }
    let (group, shape) = winner?;
    let addressing = match shape {
        OutputDelta::AppendOnlyWindow { axis } => Addressing::Window { axis: axis.clone() },
        OutputDelta::KeyedUpsert { keys } => Addressing::Keyed { keys: keys.clone() },
        OutputDelta::General { reason } => Addressing::WholeModel {
            degraded_by: reason.clone(),
        },
    };
    let degrading_group = matches!(shape, OutputDelta::General { .. }).then(|| group.clone());
    Some(SignatureHeadline {
        emits: shape.clone(),
        addressing,
        degrading_group,
        grain_label,
        run_shape,
        locality_bound,
    })
}

impl SignatureHeadline {
    /// The `emits:` clause's shape description, e.g. `"keyed upsert over
    /// [order_id]"`, `"append-only within a window"`, `"general change"`.
    pub fn emits_label(&self) -> String {
        match &self.emits {
            OutputDelta::KeyedUpsert { keys } => format!("keyed upsert over [{}]", keys.join(", ")),
            OutputDelta::AppendOnlyWindow { .. } => "append-only within a window".to_string(),
            OutputDelta::General { .. } => "general change".to_string(),
        }
    }

    /// The addressing description, e.g. `"key-addressed"`,
    /// `"window-addressed by order_date"`, or `"whole-table-addressed
    /// (<reason>[, forced by column group <group>])"` for a general/degraded
    /// emits.
    pub fn addressing_label(&self) -> String {
        match &self.addressing {
            Addressing::Keyed { .. } => "key-addressed".to_string(),
            Addressing::Window { axis } => format!("window-addressed by {axis}"),
            Addressing::WholeModel { degraded_by } => {
                let mut label = format!("whole-table-addressed ({degraded_by}");
                if let Some(group) = &self.degrading_group {
                    label.push_str(&format!(", forced by column group {group}"));
                }
                label.push(')');
                label
            }
        }
    }

    /// The derived run shape's label, if this headline carries one.
    pub fn run_shape_label(&self) -> Option<String> {
        self.run_shape.as_ref().map(KeyedRunShape::label)
    }

    /// The full single-line headline: `smelt explain`'s report's first line
    /// (text and `--json` render the SAME fields this returns — see
    /// [`Self::emits_label`]/[`Self::addressing_label`]/[`Self::run_shape_label`]/
    /// [`Self::locality_bound`] for the JSON-side per-field equivalents).
    pub fn render(&self) -> String {
        let mut out = format!(
            "emits: {}, {}, grain: {}",
            self.emits_label(),
            self.addressing_label(),
            self.grain_label
        );
        if let Some(run_shape) = self.run_shape_label() {
            out.push_str(&format!(", run shape: {run_shape}"));
        }
        if let Some(bound) = &self.locality_bound {
            out.push_str(&format!(", locality: {bound}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_model_headline_names_keys_and_key_addressing() {
        let verdicts = vec![(
            "{order_id, amount}".to_string(),
            OutputDelta::KeyedUpsert {
                keys: vec!["order_id".to_string()],
            },
        )];
        let headline =
            derive_signature_headline(&verdicts, "keyed".to_string(), None, None).unwrap();
        assert_eq!(
            headline.render(),
            "emits: keyed upsert over [order_id], key-addressed, grain: keyed"
        );
    }

    #[test]
    fn windowed_model_headline_names_the_axis() {
        let verdicts = vec![(
            "{amount, order_date}".to_string(),
            OutputDelta::AppendOnlyWindow {
                axis: "order_date".to_string(),
            },
        )];
        let headline =
            derive_signature_headline(&verdicts, "partition".to_string(), None, None).unwrap();
        assert_eq!(
            headline.render(),
            "emits: append-only within a window, window-addressed by order_date, grain: partition"
        );
    }

    #[test]
    fn general_model_headline_is_whole_table_addressed_and_names_the_cause() {
        let verdicts = vec![(
            "{weight}".to_string(),
            OutputDelta::General {
                reason: "'dims' is a mutable snapshot".to_string(),
            },
        )];
        let headline =
            derive_signature_headline(&verdicts, "partition".to_string(), None, None).unwrap();
        let rendered = headline.render();
        assert!(
            rendered.starts_with(
                "emits: general change, whole-table-addressed ('dims' is a mutable snapshot"
            ),
            "got {rendered:?}"
        );
    }

    #[test]
    fn no_derivable_verdict_prints_no_signature() {
        assert!(derive_signature_headline(&[], "partition".to_string(), None, None).is_none());

        let mixed = vec![
            (
                "{amount}".to_string(),
                OutputDelta::AppendOnlyWindow {
                    axis: "event_date".to_string(),
                },
            ),
            (
                "{weight}".to_string(),
                OutputDelta::General {
                    reason: "'dims' is a mutable snapshot".to_string(),
                },
            ),
        ];
        let headline =
            derive_signature_headline(&mixed, "partition".to_string(), None, None).unwrap();
        assert!(
            matches!(headline.emits, OutputDelta::General { .. }),
            "the mixed meet must widen to the General sibling"
        );
        assert_eq!(
            headline.degrading_group.as_deref(),
            Some("{weight}"),
            "the degrading group must be named, not just the reason"
        );
        assert!(headline
            .render()
            .contains("forced by column group {weight}"));
    }

    #[test]
    fn composed_model_headline_appends_the_locality_bound() {
        let verdicts = vec![(
            "{order_id}".to_string(),
            OutputDelta::KeyedUpsert {
                keys: vec!["order_id".to_string()],
            },
        )];
        let headline = derive_signature_headline(
            &verdicts,
            "keyed".to_string(),
            None,
            Some("route 1 (key-embedded)".to_string()),
        )
        .unwrap();
        assert!(
            headline
                .render()
                .ends_with("locality: route 1 (key-embedded)"),
            "got {:?}",
            headline.render()
        );
    }
}
