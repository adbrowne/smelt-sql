//! Phase 4 (`docs/outcomes/20260904-state-residency/outcome.md`) — availability
//! resolution, step 2 of `docs/specs/state.md` §"The degradation contract".
//! Pure-function coverage of `smelt_logical::maintenance::availability`; no
//! consumer wires this in yet (phase 5).

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, PartitionLocal, PlanCell, RowIdentity,
    RowIdentityVerdict, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

mod resolution;
mod succession;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A keyed-fold plan: `payments` is append-only, the combiner (`SUM`) is
/// invertible, so `derive_maintenance_plan` admits a `KeyedFold` cell for the
/// creation trigger (mirrors `maintenance_plan_admission.rs::inputs`).
fn keyed_fold_plan() -> smelt_logical::maintenance::MaintenancePlan {
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend \
              FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![SourceFacts {
            name: "payments".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    derive_maintenance_plan(&inputs, &[trigger])
}

fn base_cell(corner: Corner, technique: Technique) -> PlanCell {
    PlanCell {
        group: "{amount}".to_string(),
        trigger: Trigger::NewData {
            source: "payments".to_string(),
        },
        corner,
        technique,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    }
}
