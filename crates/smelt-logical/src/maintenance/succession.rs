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

use crate::analysis::succession::{SuccessionAdvisory, SuccessionVerdict};

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
/// `advisories` carries the classifier's `Recognized`-verdict advisories
/// (`SuccessionPreFilterNegatesFlag`) verbatim, empty on `NotSuccession` —
/// kept off [`MaintenancePlan`] itself so "the advisory never changes
/// admission" is a structural fact this type cannot express otherwise
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/03a-plan.md`).
pub struct SuccessionDerivation {
    pub output: OutputSpec,
    pub plan: MaintenancePlan,
    pub advisories: Vec<SuccessionAdvisory>,
    /// Every argument the four `maintenance::emit::succession` emitters take
    /// beyond the caller-supplied window predicate, presented table and
    /// dialect — `None` on a `NotSuccession` verdict, since there is nothing
    /// to maintain. The single owner of the emitters' inputs
    /// (`CLAUDE.md` §"Maintenance-plan purity"): a consumer (the runtime
    /// driver, `smelt-db`) takes this recipe, never the model's SQL.
    pub recipe: Option<SuccessionRecipe>,
}

/// Every emitter input the keyed-succession classifier's verdict already
/// holds, assembled once so no consumer re-parses the model's SQL
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/05a-plan.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessionRecipe {
    /// The driving source's comparison spelling
    /// (`SuccessionVerdict::Recognized::source`), suitable as the emitters'
    /// `source_table` argument once the caller resolves it to a physical
    /// name (the runtime driver's job, not this recipe's).
    pub source_table: String,
    pub pre_filter: Option<String>,
    pub key_cols: Vec<String>,
    pub clock_col: String,
    /// Row-local, non-key/non-clock/non-derived columns the presented table
    /// stores — `row_local` aliases minus `key_cols`, `clock_col`, and every
    /// `lead_derived`/`lag_derived` alias, in the model's own projection
    /// order.
    pub payload_columns: Vec<String>,
    /// The full row-local projection (`(alias, source expr)`), in the
    /// model's own column order — feeds `emit_succession_event_delta`'s
    /// `row_local_projection` argument directly.
    pub row_local_projection: Vec<(String, String)>,
    pub lead_derived: Vec<(String, String)>,
    pub lag_derived: Vec<(String, String)>,
    pub delete_flag_expr: Option<String>,
}

impl SuccessionRecipe {
    /// Assemble the recipe from a `Recognized` verdict. Pure: no I/O, no
    /// re-derivation of anything the verdict does not already carry.
    pub fn from_verdict(verdict: &SuccessionVerdict) -> Option<Self> {
        let SuccessionVerdict::Recognized {
            source,
            pre_filter,
            key_cols,
            clock_col,
            delete_flag_expr,
            row_local,
            lead_derived,
            lag_derived,
            ..
        } = verdict
        else {
            return None;
        };
        let excluded: BTreeSet<&str> = key_cols
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(clock_col.as_str()))
            .chain(lead_derived.iter().map(|(alias, _)| alias.as_str()))
            .chain(lag_derived.iter().map(|(alias, _)| alias.as_str()))
            .collect();
        let payload_columns = row_local
            .iter()
            .filter(|(alias, _)| !excluded.contains(alias.as_str()))
            .map(|(alias, _)| alias.clone())
            .collect();
        Some(SuccessionRecipe {
            source_table: source.clone(),
            pre_filter: pre_filter.clone(),
            key_cols: key_cols.clone(),
            clock_col: clock_col.clone(),
            payload_columns,
            row_local_projection: (**row_local).clone(),
            lead_derived: (**lead_derived).clone(),
            lag_derived: (**lag_derived).clone(),
            delete_flag_expr: (**delete_flag_expr).clone(),
        })
    }
}

/// Derive the one-cell succession plan (or the refusal plan) from the
/// classifier's verdict. Pure: no I/O, no re-derivation of `verdict` itself.
pub fn derive_succession_plan(verdict: &SuccessionVerdict, table: &str) -> SuccessionDerivation {
    match verdict {
        SuccessionVerdict::Recognized {
            source,
            key_cols,
            clock_col,
            advisories,
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
                advisories: advisories.clone(),
                recipe: SuccessionRecipe::from_verdict(verdict),
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
            advisories: Vec::new(),
            recipe: None,
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
            row_local: Box::new(vec![
                ("customer_id".to_string(), "customer_id".to_string()),
                ("changed_at".to_string(), "changed_at".to_string()),
            ]),
            lead_derived: Box::new(vec![("next_changed_at".to_string(), "{lead}".to_string())]),
            lag_derived: Box::new(vec![]),
            delete_flag_expr: Box::new(None),
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
        assert_eq!(
            derivation.advisories,
            vec![SuccessionAdvisory::PreFilterNegatesFlag {
                column: "is_deleted".to_string()
            }]
        );
    }

    /// The advisory is carried on `SuccessionDerivation`, never on
    /// `MaintenancePlan` — deriving over the same `Recognized` verdict with
    /// and without the advisory must yield byte-identical `plan` and
    /// `output`, proving admission cannot depend on it.
    #[test]
    fn advisory_does_not_change_the_derived_plan() {
        let with_advisory = recognized(vec![SuccessionAdvisory::PreFilterNegatesFlag {
            column: "is_deleted".to_string(),
        }]);
        let without_advisory = recognized(vec![]);
        let with_derivation = derive_succession_plan(&with_advisory, "customer_history");
        let without_derivation = derive_succession_plan(&without_advisory, "customer_history");
        assert_eq!(
            with_derivation.plan.cells.len(),
            without_derivation.plan.cells.len()
        );
        assert_eq!(
            with_derivation.plan.refusals.len(),
            without_derivation.plan.refusals.len()
        );
        let (with_cell, without_cell) = (
            &with_derivation.plan.cells[0],
            &without_derivation.plan.cells[0],
        );
        assert_eq!(with_cell.trigger, without_cell.trigger);
        assert_eq!(with_cell.technique, without_cell.technique);
        assert_eq!(with_cell.corner, without_cell.corner);
        assert_eq!(with_cell.row_identity, without_cell.row_identity);
        assert_eq!(
            with_derivation.output.grain,
            without_derivation.output.grain
        );
        assert_eq!(
            with_derivation.output.skeleton_columns,
            without_derivation.output.skeleton_columns
        );
        assert_ne!(with_derivation.advisories, without_derivation.advisories);
    }

    #[test]
    fn recipe_payload_columns_exclude_key_clock_and_derived() {
        let verdict = SuccessionVerdict::Recognized {
            source: "customer_changes".to_string(),
            pre_filter: None,
            key_cols: vec!["customer_id".to_string()],
            clock_col: "changed_at".to_string(),
            lead_cols: vec!["valid_to".to_string()],
            lag_cols: vec![],
            delete_flag: None,
            advisories: vec![],
            row_local: Box::new(vec![
                ("customer_id".to_string(), "customer_id".to_string()),
                ("changed_at".to_string(), "changed_at".to_string()),
                ("region".to_string(), "region".to_string()),
                ("is_deleted".to_string(), "is_deleted".to_string()),
            ]),
            lead_derived: Box::new(vec![("valid_to".to_string(), "{lead}".to_string())]),
            lag_derived: Box::new(vec![(
                "is_current".to_string(),
                "{lead} IS NULL".to_string(),
            )]),
            delete_flag_expr: Box::new(None),
        };
        let recipe = SuccessionRecipe::from_verdict(&verdict).expect("recipe on Recognized");
        assert_eq!(
            recipe.payload_columns,
            vec!["region".to_string(), "is_deleted".to_string()]
        );
    }

    #[test]
    fn recipe_is_none_for_not_succession() {
        let verdict = SuccessionVerdict::NotSuccession {
            reason: NotSuccessionReason::PatternUnrecognized("no window at all".to_string()),
        };
        assert!(SuccessionRecipe::from_verdict(&verdict).is_none());
    }

    #[test]
    fn derive_succession_plan_carries_the_recipe_on_recognition() {
        let verdict = recognized(vec![]);
        let derivation = derive_succession_plan(&verdict, "customer_history");
        let recipe = derivation.recipe.expect("recipe on Recognized");
        assert_eq!(recipe.source_table, "customer_changes");
        assert_eq!(recipe.key_cols, vec!["customer_id".to_string()]);
        assert_eq!(recipe.clock_col, "changed_at");
    }

    #[test]
    fn derive_succession_plan_recipe_is_none_on_refusal() {
        let reason = NotSuccessionReason::PatternUnrecognized("no window at all".to_string());
        let verdict = SuccessionVerdict::NotSuccession { reason };
        let derivation = derive_succession_plan(&verdict, "customer_history");
        assert!(derivation.recipe.is_none());
    }

    /// The `SuccessionPreFilterNegatesFlag` advisory never suppresses or
    /// alters the recipe (the advisory never changes admission).
    #[test]
    fn advisory_only_model_still_yields_recipe() {
        let with_advisory = recognized(vec![SuccessionAdvisory::PreFilterNegatesFlag {
            column: "is_deleted".to_string(),
        }]);
        let without_advisory = recognized(vec![]);
        let with_derivation = derive_succession_plan(&with_advisory, "customer_history");
        let without_derivation = derive_succession_plan(&without_advisory, "customer_history");
        assert_eq!(with_derivation.recipe, without_derivation.recipe);
    }
}
