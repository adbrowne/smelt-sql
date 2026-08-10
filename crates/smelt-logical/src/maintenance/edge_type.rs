//! Edge typing in the propagation layer (`docs/specs/incremental_models.md`
//! §"The graph layer" → "Typed edges"): a propagation edge carries a
//! **vector** of typed components, one per upstream column group the
//! downstream consumer reads — `(delta shape × addressing × column set)`,
//! the upstream's [`crate::analysis::output_delta::OutputDelta`] verdict
//! projected through the consumer's own per-column sensitivity. Today's
//! day-interval dirt is exactly the `AppendOnlyWindow` component under
//! window addressing (`crate::maintenance::propagate`'s `Edge`, unchanged
//! by this module); this module derives the vector but does not yet act on
//! it — `propagate`/`required_inputs` stay on interval math alone this
//! phase (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase 3).
//!
//! **Widen-never-narrow.** A component whose shape cannot be addressed
//! through the consumer (an unprojectable axis, or the upstream shape is
//! already `General`) degrades to [`Addressing::WholeModel`] — the
//! component is always *present*, naming the cause, never silently dropped.
//! A group the consumer does not read at all produces no component (there
//! is nothing to project).

use std::collections::BTreeSet;

use crate::analysis::output_delta::OutputDelta;
use crate::maintenance::ColumnGroup;

/// How a typed component addresses the rows/values it covers
/// (`incremental_models.md` §"The graph layer" → "Typed edges"): a window on
/// the upstream's append-only axis, a key set for a keyed-upsert group, or
/// whole-model dirt when neither addressing survives projection through the
/// consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Addressing {
    /// The upstream group's [`OutputDelta::AppendOnlyWindow`] axis, carried
    /// through into the consumer's own output columns.
    Window { axis: String },
    /// The upstream group's [`OutputDelta::KeyedUpsert`] key set.
    Keyed { keys: Vec<String> },
    /// The addressing could not be projected through the consumer (an
    /// unprojectable axis) or the upstream shape was already
    /// [`OutputDelta::General`]. `degraded_by` names the cause — never a
    /// silent drop (widen-never-narrow).
    WholeModel { degraded_by: String },
}

/// One typed component of a propagation edge: the upstream's derived
/// [`OutputDelta`] shape for one column group the consumer reads, its
/// projected [`Addressing`], and the group's own column set.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeComponent {
    /// Display name of the upstream column group this component covers
    /// (`ColumnGroup::name`), e.g. `{amount}`.
    pub group: String,
    /// The upstream's own derived output-delta shape for this group.
    pub shape: OutputDelta,
    /// The addressing this component projects through to the consumer.
    pub addressing: Addressing,
    /// The upstream group's own columns (unfiltered — the whole group, not
    /// just the subset the consumer happens to read).
    pub columns: Vec<String>,
}

/// Derive the typed component vector for one edge `upstream → consumer`
/// (`incremental_models.md` §"The graph layer" → "Typed edges"):
///
/// - An upstream group whose columns are entirely outside
///   `consumer_read_columns` contributes no component — there is nothing for
///   the consumer to project (test: `groups_the_consumer_does_not_read_are_omitted`).
/// - `OutputDelta::AppendOnlyWindow{axis}` projects to `Addressing::Window`
///   when `axis` is carried into one of the consumer's own derived column
///   groups (`consumer_groups`) — i.e. the consumer's own output still
///   carries that axis column, so a downstream edge can still address dirt
///   by it. When the axis is dropped by the consumer's own projection, the
///   component degrades to `Addressing::WholeModel`, naming the axis.
/// - `OutputDelta::KeyedUpsert{keys}` projects to `Addressing::Keyed`
///   unconditionally — a key set needs no carriage check the way a window
///   axis does; the consumer either reads the keyed group's columns (and so
///   is itself addressed by that key) or the group was already omitted
///   above.
/// - `OutputDelta::General{reason}` projects to `Addressing::WholeModel`,
///   carrying `reason` verbatim as `degraded_by`.
///
/// Components are one **per column group**, never merged/meet across groups
/// (`model_properties.md` §"Output-delta shape": per-column-group scoping is
/// what keeps one mutable group from degrading a sibling's shape) — a
/// two-group upstream where the consumer reads both groups yields two
/// independent components. The result is sorted by group name for a
/// deterministic edge rendering (`smelt explain`'s later consumer).
pub fn type_edge(
    upstream: &str,
    upstream_verdicts: &[(ColumnGroup, OutputDelta)],
    consumer_read_columns: &BTreeSet<String>,
    consumer_groups: &[ColumnGroup],
) -> Vec<EdgeComponent> {
    let mut components: Vec<EdgeComponent> = upstream_verdicts
        .iter()
        .filter(|(group, _)| {
            group
                .columns
                .iter()
                .any(|c| consumer_read_columns.contains(c))
        })
        .map(|(group, shape)| {
            let addressing = match shape {
                OutputDelta::AppendOnlyWindow { axis } => {
                    let carried = consumer_groups
                        .iter()
                        .any(|cg| cg.columns.iter().any(|c| c.eq_ignore_ascii_case(axis)));
                    if carried {
                        Addressing::Window { axis: axis.clone() }
                    } else {
                        Addressing::WholeModel {
                            degraded_by: format!(
                                "'{upstream}' emits append-only-window shape addressed by axis \
                                 '{axis}', but that axis column is not carried into the \
                                 consumer's own output columns"
                            ),
                        }
                    }
                }
                OutputDelta::KeyedUpsert { keys } => Addressing::Keyed { keys: keys.clone() },
                OutputDelta::General { reason } => Addressing::WholeModel {
                    degraded_by: reason.clone(),
                },
            };
            EdgeComponent {
                group: group.name(),
                shape: shape.clone(),
                addressing,
                columns: group.columns.clone(),
            }
        })
        .collect();
    components.sort_by(|a, b| a.group.cmp(&b.group));
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn group(columns: &[&str]) -> ColumnGroup {
        ColumnGroup {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }
    }

    fn read_columns(cols: &[&str]) -> BTreeSet<String> {
        cols.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn window_component_from_append_only_upstream() {
        let upstream_verdicts = vec![(
            group(&["amount", "event_date"]),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string(),
            },
        )];
        let consumer_read = read_columns(&["amount", "event_date"]);
        let consumer_groups = vec![group(&["amount", "event_date"])];

        let components = type_edge(
            "events",
            &upstream_verdicts,
            &consumer_read,
            &consumer_groups,
        );

        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].addressing,
            Addressing::Window {
                axis: "event_date".to_string()
            }
        );
    }

    #[test]
    fn keyed_component_from_keyed_upsert_upstream() {
        let upstream_verdicts = vec![(
            group(&["user_id", "n"]),
            OutputDelta::KeyedUpsert {
                keys: vec!["user_id".to_string()],
            },
        )];
        let consumer_read = read_columns(&["user_id", "n"]);
        let consumer_groups = vec![group(&["user_id", "n"])];

        let components = type_edge(
            "events",
            &upstream_verdicts,
            &consumer_read,
            &consumer_groups,
        );

        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].addressing,
            Addressing::Keyed {
                keys: vec!["user_id".to_string()]
            }
        );
        assert_eq!(components[0].columns, vec!["user_id", "n"]);
    }

    #[test]
    fn general_component_degrades_to_whole_model() {
        let upstream_verdicts = vec![(
            group(&["weight"]),
            OutputDelta::General {
                reason: "'dims' is a mutable snapshot".to_string(),
            },
        )];
        let consumer_read = read_columns(&["weight"]);
        let consumer_groups = vec![group(&["weight"])];

        let components = type_edge("dims", &upstream_verdicts, &consumer_read, &consumer_groups);

        assert_eq!(
            components.len(),
            1,
            "the component must be present, not dropped"
        );
        assert_eq!(
            components[0].addressing,
            Addressing::WholeModel {
                degraded_by: "'dims' is a mutable snapshot".to_string()
            }
        );
    }

    #[test]
    fn component_per_group_not_per_model() {
        let upstream_verdicts = vec![
            (
                group(&["amount"]),
                OutputDelta::AppendOnlyWindow {
                    axis: "event_date".to_string(),
                },
            ),
            (
                group(&["weight"]),
                OutputDelta::General {
                    reason: "'dims' is a mutable snapshot".to_string(),
                },
            ),
        ];
        let consumer_read = read_columns(&["amount", "weight", "event_date"]);
        let consumer_groups = vec![group(&["amount", "weight", "event_date"])];

        let components = type_edge(
            "mixed",
            &upstream_verdicts,
            &consumer_read,
            &consumer_groups,
        );

        assert_eq!(
            components.len(),
            2,
            "each group must yield its own component"
        );
        let amount = components
            .iter()
            .find(|c| c.columns == vec!["amount".to_string()])
            .expect("amount component present");
        let weight = components
            .iter()
            .find(|c| c.columns == vec!["weight".to_string()])
            .expect("weight component present");
        assert!(
            matches!(amount.addressing, Addressing::Window { .. }),
            "the general sibling group must not degrade the append-only group's addressing"
        );
        assert!(matches!(weight.addressing, Addressing::WholeModel { .. }));
    }

    #[test]
    fn groups_the_consumer_does_not_read_are_omitted() {
        let upstream_verdicts = vec![(
            group(&["untouched"]),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string(),
            },
        )];
        let consumer_read = read_columns(&["something_else"]);
        let consumer_groups = vec![group(&["something_else"])];

        let components = type_edge(
            "events",
            &upstream_verdicts,
            &consumer_read,
            &consumer_groups,
        );

        assert!(
            components.is_empty(),
            "a group the consumer never reads must not produce a component"
        );
    }

    #[test]
    fn unprojectable_axis_degrades_to_whole_model() {
        let upstream_verdicts = vec![(
            group(&["amount", "event_date"]),
            OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string(),
            },
        )];
        // The consumer reads 'amount' but its own output columns drop
        // 'event_date' entirely — the axis is not carried forward.
        let consumer_read = read_columns(&["amount"]);
        let consumer_groups = vec![group(&["amount"])];

        let components = type_edge(
            "events",
            &upstream_verdicts,
            &consumer_read,
            &consumer_groups,
        );

        assert_eq!(components.len(), 1);
        match &components[0].addressing {
            Addressing::WholeModel { degraded_by } => {
                assert!(
                    degraded_by.contains("event_date"),
                    "the degrade reason must name the unprojectable axis, got {degraded_by:?}"
                );
            }
            other => panic!("expected WholeModel degrade, got {other:?}"),
        }
    }

    #[test]
    fn components_are_deterministically_ordered() {
        let upstream_verdicts = vec![
            (
                group(&["zeta"]),
                OutputDelta::KeyedUpsert {
                    keys: vec!["id".to_string()],
                },
            ),
            (
                group(&["alpha"]),
                OutputDelta::AppendOnlyWindow {
                    axis: "event_date".to_string(),
                },
            ),
        ];
        let consumer_read = read_columns(&["zeta", "alpha", "event_date"]);
        let consumer_groups = vec![group(&["zeta", "alpha", "event_date"])];

        let components = type_edge("s", &upstream_verdicts, &consumer_read, &consumer_groups);
        let names: Vec<&str> = components.iter().map(|c| c.group.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "components must be sorted by group name");
    }
}
