//! TDD tests for the retention verdict → outcome mapping
//! (`docs/specs/model_properties.md` §"Reach versus retained history"):
//! `smelt_logical::maintenance::retention_outcomes` and the new
//! `derive_maintenance_plan_with_referential_integrity_and_retentions` entry
//! point that folds it into a derived plan. `crates/smelt-logical/src/
//! analysis/retention_reach.rs` already covers the proof half (deriving a
//! `RetentionVerdict` from SQL); this file covers the action half — what a
//! plan derivation does with that verdict.

use std::collections::{BTreeSet, HashMap};

use smelt_logical::analysis::retention_reach::{RetentionVerdict, UnprovableReason};
use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::derive::{
    derive_maintenance_plan, derive_maintenance_plan_with_referential_integrity_and_retentions,
    ModelInputs, SourceReferentialIntegrity, SourceRetentions,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts,
};

const SQL_30_DAY_LOOKBACK: &str = "SELECT event_date, COUNT(*) AS cnt FROM smelt.sources.events \
     WHERE event_date >= CURRENT_DATE - INTERVAL '30 days' GROUP BY event_date";

const SQL_1_DAY_LOOKBACK: &str = "SELECT event_date, COUNT(*) AS cnt FROM smelt.sources.events \
     WHERE event_date >= CURRENT_DATE - INTERVAL '1 days' GROUP BY event_date";

const SQL_UNBOUNDED: &str = "SELECT event_date, \
     SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date \
         RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running \
     FROM smelt.sources.events";

fn inputs(sql: &str) -> ModelInputs<'_> {
    ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![SourceFacts {
            name: "events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: true,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["cnt".to_string(), "running".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    }
}

fn retentions(source: &str, interval: &str) -> SourceRetentions {
    let mut r = SourceRetentions::new();
    r.insert(
        source.to_string(),
        smelt_core::config::DataLatency::parse(interval).unwrap(),
    );
    r
}

#[test]
fn exceeding_reach_refuses_with_source_retention_exceeded() {
    let plan = derive_maintenance_plan_with_referential_integrity_and_retentions(
        &inputs(SQL_30_DAY_LOOKBACK),
        &[],
        &SourceReferentialIntegrity::new(),
        &retentions("events", "7 days"),
    );
    assert_eq!(
        plan.refusals,
        vec![Refusal::SourceRetentionExceeded {
            source: "events".to_string(),
            required_lookback_secs: Seconds::days(30).0,
            retained_secs: Seconds::days(7).0,
        }],
        "a 30-day lookback over a 7-day retained source must refuse: {:?}",
        plan.refusals
    );
    assert!(
        plan.retention_downgrades.is_empty(),
        "an exceeding reach is a refusal, never also a downgrade: {:?}",
        plan.retention_downgrades
    );
}

#[test]
fn reach_within_retention_records_nothing() {
    let plan = derive_maintenance_plan_with_referential_integrity_and_retentions(
        &inputs(SQL_1_DAY_LOOKBACK),
        &[],
        &SourceReferentialIntegrity::new(),
        &retentions("events", "7 days"),
    );
    assert!(
        plan.refusals
            .iter()
            .all(|r| !matches!(r, Refusal::SourceRetentionExceeded { .. })),
        "an honoured bound must not refuse: {:?}",
        plan.refusals
    );
    assert!(
        plan.retention_downgrades.is_empty(),
        "an honoured bound is not news — it must record no downgrade: {:?}",
        plan.retention_downgrades
    );
}

#[test]
fn unprovable_reach_records_a_retention_downgrade() {
    let plan = derive_maintenance_plan_with_referential_integrity_and_retentions(
        &inputs(SQL_UNBOUNDED),
        &[],
        &SourceReferentialIntegrity::new(),
        &retentions("events", "45 days"),
    );
    assert!(
        plan.refusals
            .iter()
            .all(|r| !matches!(r, Refusal::SourceRetentionExceeded { .. })),
        "an unprovable reach is a downgrade, never a refusal: {:?}",
        plan.refusals
    );
    assert_eq!(plan.retention_downgrades.len(), 1);
    let downgrade = &plan.retention_downgrades[0];
    assert_eq!(downgrade.source, "events");
    assert_eq!(downgrade.retained, Seconds::days(45));
    assert_eq!(downgrade.reason, UnprovableReason::UnboundedReach);
}

#[test]
fn no_declared_retention_leaves_the_plan_unchanged() {
    let baseline = derive_maintenance_plan(&inputs(SQL_30_DAY_LOOKBACK), &[]);
    let with_empty_retentions = derive_maintenance_plan_with_referential_integrity_and_retentions(
        &inputs(SQL_30_DAY_LOOKBACK),
        &[],
        &SourceReferentialIntegrity::new(),
        &SourceRetentions::new(),
    );
    assert_eq!(baseline.refusals, with_empty_retentions.refusals);
    assert_eq!(
        baseline.retention_downgrades,
        with_empty_retentions.retention_downgrades
    );
    assert!(with_empty_retentions.retention_downgrades.is_empty());
}

/// The no-silent-under-read gate: every one of the four `RetentionVerdict`
/// shapes maps to exactly a refusal, a recorded downgrade, or neither —
/// never both, and never a fifth outcome. Driven data-first over the full
/// shape set rather than sampling one variant.
#[test]
fn every_retention_verdict_maps_to_a_refusal_a_downgrade_or_an_admitted_fit() {
    let cases: Vec<(&str, RetentionVerdict, bool, bool)> = vec![
        (
            "no_declared_bound",
            RetentionVerdict::NoDeclaredBound,
            false,
            false,
        ),
        (
            "within",
            RetentionVerdict::Within {
                required_lookback: Seconds::days(1),
                retained: Seconds::days(7),
            },
            false,
            false,
        ),
        (
            "exceeds",
            RetentionVerdict::Exceeds {
                required_lookback: Seconds::days(30),
                retained: Seconds::days(7),
            },
            true,
            false,
        ),
        (
            "unprovable",
            RetentionVerdict::UnprovableWithin {
                retained: Seconds::days(45),
                reason: UnprovableReason::ReachNotDerivable,
            },
            false,
            true,
        ),
    ];
    for (name, verdict, expect_refusal, expect_downgrade) in cases {
        let mut verdicts = HashMap::new();
        verdicts.insert(name.to_string(), verdict);
        let (refusals, downgrades) = smelt_logical::maintenance::retention_outcomes(&verdicts);
        assert_eq!(
            !refusals.is_empty(),
            expect_refusal,
            "{name}: refusal presence mismatch: {refusals:?}"
        );
        assert_eq!(
            !downgrades.is_empty(),
            expect_downgrade,
            "{name}: downgrade presence mismatch: {downgrades:?}"
        );
        assert!(
            !(expect_refusal && expect_downgrade),
            "{name}: a verdict can never be both a refusal and a downgrade"
        );
    }
}
