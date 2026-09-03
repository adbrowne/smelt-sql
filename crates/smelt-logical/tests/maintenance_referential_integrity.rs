//! TDD tests for `derive_maintenance_plan_with_referential_integrity` and
//! its supporting derivations (`smelt_logical::maintenance::derive`):
//! `mutation_enrichment_closure` (the external-source analogue of
//! `append_model_edge_cells`'s P1 skeleton-source-closure proof, T3 over
//! external sources, `docs/plans/20260715-composed-axes-conditional-
//! maintenance.md` Phase F5) and `model_fingerprint_projections` (P4,
//! `model_properties.md` §"Fingerprint projection"). Neither had a direct
//! test through the public entry point before this file — every existing
//! `UpstreamMutation` fixture calls the referential-integrity-free
//! `derive_maintenance_plan`, which always threads an empty
//! `SourceReferentialIntegrity` map (`skeleton_source_closure: None` for
//! every cell), so the "an entry IS present" path went unexercised.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{
    derive_maintenance_plan, derive_maintenance_plan_with_referential_integrity, ModelInputs,
    SourceReferentialIntegrity,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, SourceFacts, Trigger,
};

/// A fact + dimension enrichment, LEFT JOIN, payload-only projection, the
/// dimension's `unique_key: [id]` declared — the closure-admissible shape
/// (mirrors `skeleton_closure.rs`'s own `left_join_payload_only_closes`
/// fixture), with `dim` as the `UpstreamMutation`-triggering external
/// source rather than a model edge.
const CLOSED_SQL: &str = "SELECT fact.event_id, fact.event_date, dim.tier \
     FROM smelt.sources.fact fact \
     LEFT JOIN smelt.sources.dim dim ON fact.dim_id = dim.id";

fn inputs() -> ModelInputs<'static> {
    ModelInputs {
        sql: CLOSED_SQL,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![
            SourceFacts {
                name: "fact".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("event_date".to_string()),
                unique_key: vec![],
                allow_full_scan: true,
            },
            SourceFacts {
                name: "dim".to_string(),
                mutation: MutationProfile::MutableSnapshot,
                partition_col: None,
                unique_key: vec!["id".to_string()],
                allow_full_scan: true,
            },
        ],
        column_groups: vec![ColumnGroup {
            columns: vec!["tier".to_string()],
            mutation_sensitivity: BTreeSet::from(["dim".to_string()]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    }
}

/// Without a `source_referential_integrity` entry for `dim`,
/// `derive_maintenance_plan` (byte-identical to threading an empty map)
/// never attempts the closure proof — `skeleton_source_closure: None`,
/// matching every `UpstreamMutation` cell's pre-Phase-F5 behaviour.
#[test]
fn no_referential_integrity_entry_skips_the_closure_proof() {
    let plan = derive_maintenance_plan(
        &inputs(),
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
    );
    assert_eq!(plan.cells[0].skeleton_source_closure, None);
}

/// With a `source_referential_integrity` entry declaring `dim`'s
/// referential integrity, `derive_maintenance_plan_with_referential_
/// integrity` attempts the same P1 skeleton-source-closure proof
/// `append_model_edge_cells` runs for model edges — a LEFT JOIN,
/// payload-only, declared-`unique_key` dimension closes, driven through
/// `mutation_enrichment_closure` + `source_facts_join_context` threading the
/// declared `unique_key` into the `JoinContext` conjunct-3 (one-to-one)
/// check.
#[test]
fn referential_integrity_entry_proves_closure_for_a_closing_join() {
    let mut ri = SourceReferentialIntegrity::new();
    ri.insert("dim".to_string(), vec!["id".to_string()]);
    let plan = derive_maintenance_plan_with_referential_integrity(
        &inputs(),
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
        &ri,
    );
    assert_eq!(
        plan.cells[0]
            .skeleton_source_closure
            .as_ref()
            .map(|c| c.is_closed()),
        Some(true),
        "a LEFT JOIN, payload-only, declared-unique_key dimension must close, got {:?}",
        plan.cells[0].skeleton_source_closure
    );
}

/// Dropping the dimension's declared `unique_key` (so `source_facts_join_
/// context` contributes no one-to-one fact) reopens the closure even with a
/// `referential_integrity` entry present — a declared `referential_
/// integrity` alone never guarantees `Closed`.
#[test]
fn referential_integrity_entry_without_declared_unique_key_stays_open() {
    let mut i = inputs();
    i.sources[1].unique_key = Vec::new();
    let mut ri = SourceReferentialIntegrity::new();
    ri.insert("dim".to_string(), vec!["id".to_string()]);
    let plan = derive_maintenance_plan_with_referential_integrity(
        &i,
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
        &ri,
    );
    assert_eq!(
        plan.cells[0]
            .skeleton_source_closure
            .as_ref()
            .map(|c| c.is_closed()),
        Some(false),
        "no declared unique_key on the enrichment join must leave conjunct 3 (one-to-one) \
         unproven, got {:?}",
        plan.cells[0].skeleton_source_closure
    );
}

/// P4 fingerprint projection (`model_properties.md` §"Fingerprint
/// projection") is derived once per model and shared onto every cell —
/// every declared source gets an entry, keyed by name.
#[test]
fn fingerprint_projections_are_populated_per_source_on_every_cell() {
    let plan = derive_maintenance_plan(
        &inputs(),
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
    );
    let projections = &plan.cells[0].fingerprint_projections;
    assert_eq!(
        projections.len(),
        2,
        "expected one projection per declared source (fact, dim), got {projections:?}"
    );
    assert!(projections.contains_key("fact"));
    assert!(projections.contains_key("dim"));
}
