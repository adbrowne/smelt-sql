//! The succession grain's pure plan deriver
//! (`docs/specs/incremental_shapes.md` §"The succession grain"): turns the
//! keyed-succession leaf classifier's verdict
//! (`crate::analysis::succession::classify_keyed_succession`) into exactly
//! one [`super::MaintenancePlan`] cell, or one
//! [`super::Refusal::SuccessionNotRecognized`] — bypassing the general
//! [`super::derive::derive_maintenance_plan`] entirely, mirroring
//! [`super::unsupported_grain_plan`]/[`super::locality_refused_plan`]'s own
//! bypass (`super::Grain::Succession` is unreachable from that deriver).

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::succession::SuccessionVerdict;

use super::{
    succession_refused_plan, Corner, Grain, MaintenancePlan, OutputSpec, PartitionLocal, PlanCell,
    RowIdentity, RowIdentityVerdict, Technique, Trigger,
};

/// The output shape plus the derived plan for one succession-shaped model
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/03-plan.md`
/// criterion 3). `output` carries [`Grain::Succession`] and the `k ∪ {t}`
/// skeleton on a `Recognized` verdict; on `NotSuccession` it is a plain
/// placeholder (empty key, no meaningful grain) since the caller only ever
/// reads `plan` in that case — `derive_model_maintenance_plan` returns the
/// refusal plan and never constructs an [`super::OutputSpec`] from it.
pub struct SuccessionDerivation {
    pub output: OutputSpec,
    pub plan: MaintenancePlan,
}

/// Derive the one-cell succession plan (or the refusal plan) from the
/// classifier's verdict. Pure: no I/O, no re-derivation of `verdict` itself.
pub fn derive_succession_plan(verdict: &SuccessionVerdict, table: &str) -> SuccessionDerivation {
    match verdict {
        SuccessionVerdict::Recognized {
            source,
            key_cols,
            clock_col,
            ..
        } => {
            let skeleton_columns: BTreeSet<String> = key_cols
                .iter()
                .cloned()
                .chain(std::iter::once(clock_col.clone()))
                .collect();
            let grain = Grain::Succession {
                key_cols: key_cols.clone(),
                clock_col: clock_col.clone(),
            };
            let output = OutputSpec {
                table: table.to_string(),
                grain,
                skeleton_columns,
            };
            // `source` carries the classifier's own comparison spelling
            // (`analysis::succession::SuccessionContext::source_name`, e.g.
            // `"sources.customer_changes"` — the dot-joined path exactly as
            // `analysis::walk::InputItem::Table::name` spells it, required
            // so rule 1's FROM-target comparison can match it verbatim).
            // Every other `Trigger::NewData` in the plan model carries the
            // BARE source name (`SourceFacts::name`,
            // `maintenance::derive::derive_triggers`) — strip the one
            // `sources.` segment here so a succession cell's trigger
            // addresses the same way as every other cell's.
            let bare_source = source.strip_prefix("sources.").unwrap_or(source);
            let cell = PlanCell {
                group: "{*}".to_string(),
                trigger: Trigger::NewData {
                    source: bare_source.to_string(),
                },
                corner: Corner::FoldDelta,
                technique: Technique::SuccessionPatch,
                // The succession grain's run axis is the driving source's
                // own arrival/event-time partitioning
                // (`incremental_shapes.md` §"Run shape and late events") —
                // always partition-local by construction of the grain
                // itself, never an unbounded scan.
                partition_local: PartitionLocal::Yes,
                scans: Vec::new(),
                ledger_catch_up: false,
                row_identity: RowIdentityVerdict {
                    identity: RowIdentity::Key(key_cols.clone()),
                    proven_mismatch: None,
                },
                skeleton_source_closure: None,
                fingerprint_projections: BTreeMap::new(),
                key_scope: None,
                state_downgrade: None,
            };
            SuccessionDerivation {
                output,
                plan: MaintenancePlan {
                    cells: vec![cell],
                    refusals: Vec::new(),
                    key_locality: None,
                },
            }
        }
        SuccessionVerdict::NotSuccession { reason } => SuccessionDerivation {
            output: OutputSpec {
                table: table.to_string(),
                grain: Grain::Key {
                    unique_key: Vec::new(),
                },
                skeleton_columns: BTreeSet::new(),
            },
            plan: succession_refused_plan(reason.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::succession::{NotSuccessionReason, SuccessionAdvisory};

    fn recognized(advisories: Vec<SuccessionAdvisory>) -> SuccessionVerdict {
        SuccessionVerdict::Recognized {
            source: "customer_changes".to_string(),
            pre_filter: None,
            key_cols: vec!["customer_id".to_string()],
            clock_col: "changed_at".to_string(),
            lead_cols: vec!["next_changed_at".to_string()],
            lag_cols: vec![],
            delete_flag: None,
            advisories,
        }
    }

    #[test]
    fn derive_succession_plan_yields_one_patch_cell() {
        let verdict = recognized(vec![]);
        let derivation = derive_succession_plan(&verdict, "customer_history");
        assert_eq!(derivation.plan.cells.len(), 1);
        assert!(derivation.plan.refusals.is_empty());
        let cell = &derivation.plan.cells[0];
        assert_eq!(
            cell.trigger,
            Trigger::NewData {
                source: "customer_changes".to_string()
            }
        );
        assert_eq!(cell.corner, Corner::FoldDelta);
        assert_eq!(cell.technique, Technique::SuccessionPatch);
        assert_eq!(cell.partition_local, PartitionLocal::Yes);
        assert_eq!(cell.state_downgrade, None);
    }

    #[test]
    fn succession_plan_skeleton_is_key_plus_clock() {
        let verdict = recognized(vec![]);
        let derivation = derive_succession_plan(&verdict, "customer_history");
        let expected: BTreeSet<String> = ["customer_id", "changed_at"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(derivation.output.skeleton_columns, expected);
        assert_eq!(
            derivation.output.grain,
            Grain::Succession {
                key_cols: vec!["customer_id".to_string()],
                clock_col: "changed_at".to_string(),
            }
        );
    }

    #[test]
    fn succession_refusal_plan_carries_the_classifier_reason() {
        let reason = NotSuccessionReason::PatternUnrecognized("no window at all".to_string());
        let verdict = SuccessionVerdict::NotSuccession {
            reason: reason.clone(),
        };
        let derivation = derive_succession_plan(&verdict, "customer_history");
        assert!(derivation.plan.cells.is_empty());
        assert_eq!(derivation.plan.refusals.len(), 1);
        match &derivation.plan.refusals[0] {
            super::super::Refusal::SuccessionNotRecognized { reason: got } => {
                assert_eq!(*got, reason);
            }
            other => panic!("expected SuccessionNotRecognized, got {other:?}"),
        }
    }

    #[test]
    fn succession_recognition_records_the_advisory() {
        let verdict = recognized(vec![SuccessionAdvisory::PreFilterNegatesFlag {
            column: "is_deleted".to_string(),
        }]);
        let derivation = derive_succession_plan(&verdict, "customer_history");
        assert_eq!(derivation.plan.cells.len(), 1);
        let cell = &derivation.plan.cells[0];
        assert_eq!(cell.technique, Technique::SuccessionPatch);
    }
}
