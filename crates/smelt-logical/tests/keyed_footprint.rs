//! Phase 26a TDD: `ScanClamp` carries the **derived** write footprint, or
//! none at all — never an assumed mirror of its own read reach
//! (`docs/specs/model_properties.md` §"Footprint reflection / bounded write
//! footprint"). A keyed-grain output poses the footprint question against
//! its declared `timeseries.partition_column` when it has one; a bare
//! keyed output (no declared axis) gets no footprint claim.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, PartitionLocal, SourceFacts, Trigger,
};
use smelt_logical::Seconds;
use smelt_types::SqlFunction;

fn payments_source() -> SourceFacts {
    SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }
}

fn keyed_fold_plan(
    sql: &str,
    keyed_time_axis: Option<&str>,
) -> smelt_logical::maintenance::MaintenancePlan {
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["user_id".to_string()],
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![payments_source()],
        column_groups: vec![ColumnGroup {
            columns: vec!["total".to_string()],
            mutation_sensitivity: BTreeSet::from(["payments".to_string()]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("total".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis,
    };
    derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "payments".to_string(),
        }],
    )
}

#[test]
fn keyed_output_with_declared_axis_carries_the_derived_footprint() {
    let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY pay_date \
               RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS total \
               FROM smelt.sources.payments";
    let plan = keyed_fold_plan(sql, Some("pay_date"));
    let cell = &plan.cells[0];
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert_eq!(cell.scans.len(), 1);
    let clamp = &cell.scans[0];
    assert_eq!(clamp.before, Seconds::days(1));
    assert_eq!(clamp.after, Seconds::ZERO);
    // The mirror: a payment at t writes output over [t, t + 1d] — the
    // derived footprint, not a re-mirror of the clamp's own read margins.
    assert_eq!(
        clamp.footprint(),
        Some((Seconds::ZERO, Seconds::days(1))),
        "a keyed output with a declared time axis must carry the derived footprint, got {:?}",
        clamp.footprint()
    );
}

#[test]
fn keyed_output_with_a_trajectory_column_refuses_the_clamp() {
    // The LAG window gives a nonzero, bounded READ margin (so a
    // pre-derivation clamp would have admitted this source); the SUM window
    // — same axis, no frame at all, the canonical running-total shape — is
    // a trajectory column, so the derived WRITE footprint is `Unbounded`.
    let sql = "SELECT user_id, \
               LAG(amount) OVER (PARTITION BY user_id ORDER BY pay_date \
                 RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS prev_amount, \
               SUM(amount) OVER (PARTITION BY user_id ORDER BY pay_date) AS total \
               FROM smelt.sources.payments";
    let plan = keyed_fold_plan(sql, Some("pay_date"));
    let cell = &plan.cells[0];
    assert!(
        cell.scans.is_empty(),
        "an unbounded derived footprint must refuse the clamp, got {:?}",
        cell.scans
    );
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, why }
            if source == "payments" && why.contains("unbounded")),
        "expected the unbounded-footprint refusal, got {:?}",
        cell.partition_local
    );
}

#[test]
fn bare_keyed_output_clamp_carries_no_footprint_claim() {
    let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY pay_date \
               RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS total \
               FROM smelt.sources.payments";
    let plan = keyed_fold_plan(sql, None);
    let cell = &plan.cells[0];
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert_eq!(cell.scans.len(), 1);
    let clamp = &cell.scans[0];
    assert_eq!(clamp.before, Seconds::days(1));
    assert_eq!(clamp.after, Seconds::ZERO);
    assert_eq!(
        clamp.footprint(),
        None,
        "a bare keyed output (no declared time axis) must carry no footprint claim, got {:?}",
        clamp.footprint()
    );
}

#[test]
fn partition_addressed_clamp_carries_the_derived_footprint_numbers() {
    let sql = "SELECT event_id, CAST(event_ts AS DATE) AS event_date \
               FROM smelt.sources.bronze_events \
               WHERE arrival_ts < event_ts + INTERVAL '48 hours' \
                 AND arrival_date BETWEEN CAST(event_ts AS DATE) \
                                      AND CAST(event_ts AS DATE) + INTERVAL '2 days'";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "silver_events".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: BTreeSet::from(["event_id".to_string(), "event_date".to_string()]),
        },
        sources: vec![SourceFacts {
            name: "bronze_events".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: Some("arrival_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["event_id".to_string(), "event_date".to_string()],
            mutation_sensitivity: BTreeSet::from(["bronze_events".to_string()]),
            membership_sensitivity: BTreeSet::from(["bronze_events".to_string()]),
        }],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    let clamp = &cell.scans[0];
    assert_eq!(clamp.column, "arrival_date");
    assert_eq!(clamp.before, Seconds::ZERO);
    assert_eq!(clamp.after, Seconds::days(2));
    // The stored footprint is the `FootprintResult::Bounded` value the
    // asymmetric read reach reflects to, asserted deliberately asymmetric
    // so a caller reading `before`/`after` swapped would fail this test.
    assert_eq!(clamp.footprint(), Some((Seconds::days(2), Seconds::ZERO)));
}
