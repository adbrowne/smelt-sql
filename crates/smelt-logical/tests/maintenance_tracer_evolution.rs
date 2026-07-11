//! Tracer bullet, series 2: one model evolving through five versions
//! (`docs/research/20260705-refresh-as-maintenance-plan/`), asserting the
//! derived plan per version — corners, partition-locality verdicts, and the
//! *derived scan clamps* that make each maintenance op efficient.
//!
//! The model: bronze events (arrival-partitioned) landed into a silver table
//! partitioned on event date.
//! - v1: bare landing — no lateness clamp, so an arrival can rewrite any
//!   event-date partition: the footprint is unbounded and the plan says so.
//! - v2: 48h late-arrival clamp with an **explicit** `arrival_date` partition
//!   predicate — the clamp becomes derivable and maintenance partition-local.
//! - v3: 48h dedup on `event_id` (dedup window ⊆ lateness window) — skeleton
//!   gains the dedup role; the plan shape is unchanged.
//! - v4: + `session_id` from a sessions lookup (max length 48h) — a payload
//!   field-add whose clamp derives from the explicit `session_start_date`
//!   predicate; without that predicate the add refuses as scan-unbounded.
//! - v5: + `conversion_score` (first score within 14d) — a payload field-add
//!   with a *forward* clamp, plus an ongoing mutation cell whose footprint is
//!   the clamp reflected.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, PartitionLocal, Refusal, ScanClamp,
    SourceFacts, Technique, Trigger,
};
use smelt_logical::Seconds;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn bronze() -> SourceFacts {
    SourceFacts {
        name: "bronze_events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("arrival_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn sessions_src() -> SourceFacts {
    SourceFacts {
        name: "sessions".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("session_start_date".to_string()),
        unique_key: strings(&["session_id"]),
        allow_full_scan: false,
    }
}

fn conversions_src() -> SourceFacts {
    SourceFacts {
        name: "conversions".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("conversion_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn output() -> OutputSpec {
    OutputSpec {
        table: "silver_events".to_string(),
        grain: Grain::Partition {
            partition_col: "event_date".to_string(),
        },
        // v3 onward: event_id carries the dedup role.
        skeleton_columns: set(&["event_id", "event_date"]),
    }
}

const V1_SQL: &str = "SELECT event_id, user_id, event_ts, arrival_ts, \
     CAST(event_ts AS DATE) AS event_date, page \
     FROM smelt.sources.bronze_events";

const V2_SQL: &str = "SELECT event_id, user_id, event_ts, arrival_ts, \
     CAST(event_ts AS DATE) AS event_date, page \
     FROM smelt.sources.bronze_events \
     WHERE arrival_ts < event_ts + INTERVAL '48 hours' \
       AND arrival_date BETWEEN CAST(event_ts AS DATE) \
                            AND CAST(event_ts AS DATE) + INTERVAL '2 days'";

const V3_SQL: &str = "SELECT event_id, user_id, event_ts, arrival_ts, \
     CAST(event_ts AS DATE) AS event_date, page \
     FROM smelt.sources.bronze_events \
     WHERE arrival_ts < event_ts + INTERVAL '48 hours' \
       AND arrival_date BETWEEN CAST(event_ts AS DATE) \
                            AND CAST(event_ts AS DATE) + INTERVAL '2 days' \
     QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_ts) = 1";

const V4_JOIN: &str = " LEFT JOIN smelt.sources.sessions s \
       ON s.user_id = e.user_id \
      AND e.event_ts >= s.session_start_ts \
      AND e.event_ts < s.session_end_ts \
      AND s.session_start_date BETWEEN CAST(e.event_ts AS DATE) - INTERVAL '2 days' \
                                   AND CAST(e.event_ts AS DATE)";

/// The v4 join with the partition predicate omitted — the shape neither smelt
/// nor the engine can prune (smelt cannot know how `session_start_ts` relates
/// to `session_start_date`).
const V4_JOIN_NO_PARTITION_PREDICATE: &str = " LEFT JOIN smelt.sources.sessions s \
       ON s.user_id = e.user_id \
      AND e.event_ts >= s.session_start_ts \
      AND e.event_ts < s.session_end_ts";

fn v4_sql(join: &str) -> String {
    format!(
        "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, \
         CAST(e.event_ts AS DATE) AS event_date, e.page, s.session_id \
         FROM smelt.sources.bronze_events e{join} \
         WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
           AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '2 days' \
         QUALIFY ROW_NUMBER() OVER (PARTITION BY e.event_id ORDER BY e.arrival_ts) = 1"
    )
}

fn v5_sql() -> String {
    format!(
        "SELECT e.event_id, e.user_id, e.event_ts, e.arrival_ts, \
         CAST(e.event_ts AS DATE) AS event_date, e.page, s.session_id, \
         (SELECT c.score FROM smelt.sources.conversions c \
           WHERE c.event_id = e.event_id \
             AND c.conversion_ts >= e.event_ts \
             AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
             AND c.conversion_date BETWEEN CAST(e.event_ts AS DATE) \
                                       AND CAST(e.event_ts AS DATE) + INTERVAL '14 days' \
           ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
         FROM smelt.sources.bronze_events e{V4_JOIN} \
         WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours' \
           AND e.arrival_date BETWEEN CAST(e.event_ts AS DATE) \
                                  AND CAST(e.event_ts AS DATE) + INTERVAL '2 days' \
         QUALIFY ROW_NUMBER() OVER (PARTITION BY e.event_id ORDER BY e.arrival_ts) = 1"
    )
}

fn base_group() -> ColumnGroup {
    ColumnGroup {
        columns: strings(&["user_id", "event_ts", "arrival_ts", "page"]),
        mutation_sensitivity: BTreeSet::new(),
    }
}

fn clamp_for<'a>(scans: &'a [ScanClamp], source: &str) -> &'a ScanClamp {
    scans
        .iter()
        .find(|c| c.source == source)
        .unwrap_or_else(|| panic!("no scan clamp for '{source}' in {scans:?}"))
}

// ---------------------------------------------------------------------------
// v1 — no lateness clamp: the write footprint on event_date is unbounded.
// ---------------------------------------------------------------------------

#[test]
fn v1_without_lateness_clamp_is_not_partition_local() {
    let inputs = ModelInputs {
        sql: V1_SQL,
        output: output(),
        sources: vec![bronze()],
        column_groups: vec![base_group()],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    let cell = &plan.cells[0];
    assert_eq!(cell.corner, Corner::RecomputeRegion);
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, why }
            if source == "bronze_events" && why.contains("no predicate links")),
        "expected the unlinked cross-axis verdict, got {:?}",
        cell.partition_local
    );
}

// ---------------------------------------------------------------------------
// v2 — the 48h clamp with an explicit arrival_date predicate: derivable,
// partition-local, and the derived clamp is exactly (before=0, after=2d).
// ---------------------------------------------------------------------------

#[test]
fn v2_lateness_clamp_derives_the_forward_arrival_scan() {
    let inputs = ModelInputs {
        sql: V2_SQL,
        output: output(),
        sources: vec![bronze()],
        column_groups: vec![base_group()],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[
            Trigger::NewData {
                source: "bronze_events".to_string(),
            },
            Trigger::Backfill,
        ],
    );
    assert!(plan.refusals.is_empty());
    for cell in &plan.cells {
        assert_eq!(cell.partition_local, PartitionLocal::Yes);
        let clamp = clamp_for(&cell.scans, "bronze_events");
        assert_eq!(clamp.column, "arrival_date");
        assert_eq!(clamp.before, Seconds::ZERO);
        assert_eq!(clamp.after, Seconds::days(2));
        // Reflection: an arrival at t writes event_date in [t − 2d, t].
        assert_eq!(clamp.footprint(), (Seconds::days(2), Seconds::ZERO));
    }
}

// ---------------------------------------------------------------------------
// v3 — dedup joins the skeleton; the plan shape and clamps are unchanged
// (the dedup window ⊆ the lateness window, so the same scan sees every
// duplicate candidate — proven end-to-end in the DuckDB leg).
// ---------------------------------------------------------------------------

#[test]
fn v3_dedup_keeps_the_v2_plan_shape() {
    let inputs = ModelInputs {
        sql: V3_SQL,
        output: output(),
        sources: vec![bronze()],
        column_groups: vec![base_group()],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "bronze_events".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty());
    let cell = &plan.cells[0];
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert_eq!(
        clamp_for(&cell.scans, "bronze_events").after,
        Seconds::days(2)
    );
}

// ---------------------------------------------------------------------------
// v4 — + session_id: payload field-add, bottom-left column MERGE with the
// 48h-max-session lookback derived from the explicit session_start_date
// predicate; omitting that predicate refuses.
// ---------------------------------------------------------------------------

fn v4_inputs(sql: &str) -> ModelInputs<'_> {
    ModelInputs {
        sql,
        output: output(),
        sources: vec![bronze(), sessions_src()],
        column_groups: vec![
            base_group(),
            ColumnGroup {
                columns: strings(&["session_id"]),
                mutation_sensitivity: set(&["sessions"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    }
}

#[test]
fn v4_session_field_add_derives_the_two_day_lookback() {
    let sql = v4_sql(V4_JOIN);
    let inputs = v4_inputs(&sql);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["session_id"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.group, "{session_id}");
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert!(cell.ledger_catch_up);
    let clamp = clamp_for(&cell.scans, "sessions");
    assert_eq!(clamp.column, "session_start_date");
    assert_eq!(clamp.before, Seconds::days(2));
    assert_eq!(clamp.after, Seconds::ZERO);
}

#[test]
fn v4_without_the_explicit_partition_predicate_refuses_scan_unbounded() {
    // smelt cannot know how session_start_ts relates to session_start_date
    // (and the engine cannot prune on it either) — without the explicit
    // predicate the field-add must refuse, not silently full-scan sessions.
    let sql = v4_sql(V4_JOIN_NO_PARTITION_PREDICATE);
    let inputs = v4_inputs(&sql);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["session_id"]),
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    assert!(
        matches!(&plan.refusals[..], [Refusal::ScanUnbounded { source, .. }] if source == "sessions"),
        "refusals: {:?}",
        plan.refusals
    );
}

// ---------------------------------------------------------------------------
// v5 — + conversion_score: forward 14d clamp on the field-add backfill, and
// an ongoing mutation cell whose footprint is the clamp reflected.
// ---------------------------------------------------------------------------

fn v5_inputs(sql: &str) -> ModelInputs<'_> {
    ModelInputs {
        sql,
        output: output(),
        sources: vec![bronze(), sessions_src(), conversions_src()],
        column_groups: vec![
            base_group(),
            ColumnGroup {
                columns: strings(&["session_id"]),
                mutation_sensitivity: set(&["sessions"]),
            },
            ColumnGroup {
                columns: strings(&["conversion_score"]),
                mutation_sensitivity: set(&["conversions"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    }
}

#[test]
fn v5_conversion_field_add_derives_the_forward_fourteen_day_scan() {
    let sql = v5_sql();
    let inputs = v5_inputs(&sql);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["conversion_score"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 1, "only the added group gets an op");
    let cell = &plan.cells[0];
    assert_eq!(cell.group, "{conversion_score}");
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert!(cell.ledger_catch_up);
    let clamp = clamp_for(&cell.scans, "conversions");
    assert_eq!(clamp.column, "conversion_date");
    assert_eq!(clamp.before, Seconds::ZERO);
    assert_eq!(clamp.after, Seconds::days(14));
}

#[test]
fn v5_ongoing_conversion_arrival_has_the_reflected_footprint() {
    let sql = v5_sql();
    let inputs = v5_inputs(&sql);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "conversions".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.group, "{conversion_score}");
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert!(!cell.ledger_catch_up, "ongoing repair, not a catch-up");
    // Scan (0, 14d) reflects to footprint (14d, 0): a conversion at t
    // repairs events over [t − 14d, t].
    let clamp = clamp_for(&cell.scans, "conversions");
    assert_eq!(clamp.footprint(), (Seconds::days(14), Seconds::ZERO));
}

#[test]
fn v4_and_v5_adds_are_payload_but_a_dedup_key_add_refuses() {
    let sql = v5_sql();
    let mut inputs = v5_inputs(&sql);
    inputs
        .output
        .skeleton_columns
        .insert("device_id".to_string());
    // Adding a field that lands in the dedup/identity skeleton is a grain
    // change and refuses — the contrast that keeps v4/v5 honest.
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["device_id"]),
        }],
    );
    assert!(
        matches!(&plan.refusals[..], [Refusal::SkeletonColumnAdded { column }] if column == "device_id")
    );
}
