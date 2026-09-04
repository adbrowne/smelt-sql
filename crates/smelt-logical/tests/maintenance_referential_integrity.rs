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
    append_model_edge_cells, derive_maintenance_plan,
    derive_maintenance_plan_with_referential_integrity, ModelEdge, ModelInputs,
    SourceReferentialIntegrity,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MaintenancePlan, MutationProfile, OutputSpec, RowIdentity, SourceFacts,
    Trigger,
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

// --- Phase 5: model-edge cells consult declared-RI/unique-key facts for
// their own external-source enrichment joins too, not only their upstream
// model edges (`docs/outcomes/20260904-walk-migration-residue/outcome.md`
// phase 5, `model_properties.md` §"Skeleton-source closure").

fn silver_fact_edge() -> ModelEdge {
    ModelEdge {
        name: "silver.fact".to_string(),
        clock_col: Some("event_date".to_string()),
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: None,
    }
}

/// A downstream over a model edge (the driving `FROM`) plus an inner-joined
/// external dimension source declaring both `unique_key` and
/// `referential_integrity`.
const EDGE_PLUS_INNER_JOINED_DIM_SQL: &str = "SELECT fact.event_id, fact.event_date, dim.tier \
     FROM smelt.silver.fact fact \
     JOIN smelt.sources.dim dim ON fact.dim_id = dim.id";

fn dim_source_with_unique_key() -> SourceFacts {
    SourceFacts {
        name: "dim".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: None,
        unique_key: vec!["id".to_string()],
        allow_full_scan: true,
    }
}

#[test]
fn model_edge_cell_closure_consults_declared_source_ri() {
    let mut plan_without_ri = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan_without_ri,
        EDGE_PLUS_INNER_JOINED_DIM_SQL,
        Some("event_date"),
        &[silver_fact_edge()],
        &[],
        &[dim_source_with_unique_key()],
        &SourceReferentialIntegrity::new(),
    );
    assert_eq!(
        plan_without_ri.cells[0]
            .skeleton_source_closure
            .as_ref()
            .map(|c| c.is_closed()),
        Some(false),
        "an inner-joined external source with no declared referential_integrity must stay Open, \
         got {:?}",
        plan_without_ri.cells[0].skeleton_source_closure
    );

    let mut ri = SourceReferentialIntegrity::new();
    ri.insert("dim".to_string(), vec!["id".to_string()]);
    let mut plan_with_ri = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan_with_ri,
        EDGE_PLUS_INNER_JOINED_DIM_SQL,
        Some("event_date"),
        &[silver_fact_edge()],
        &[],
        &[dim_source_with_unique_key()],
        &ri,
    );
    assert_eq!(
        plan_with_ri.cells[0]
            .skeleton_source_closure
            .as_ref()
            .map(|c| c.is_closed()),
        Some(true),
        "a declared referential_integrity plus a declared unique_key on the inner-joined \
         dimension must close, got {:?}",
        plan_with_ri.cells[0].skeleton_source_closure
    );
}

#[test]
fn model_edge_closure_open_when_external_inner_join_unproven() {
    // Two model edges: the driving `fact` (no enrichment join of its own)
    // and a LEFT JOIN edge `edge2` that trivially closes via join shape —
    // "every model edge is closed" — plus an inner-joined external `dim`
    // source with NO referential_integrity entry at all.
    let sql = "SELECT fact.event_id, fact.event_date, edge2.val, dim.tier \
        FROM smelt.silver.fact fact \
        LEFT JOIN smelt.silver.edge2 edge2 ON fact.id = edge2.fact_id \
        JOIN smelt.sources.dim dim ON fact.dim_id = dim.id";
    let edge2 = ModelEdge {
        name: "silver.edge2".to_string(),
        clock_col: Some("event_date".to_string()),
        clock_col_aliases: vec![],
        unique_key: vec!["id".to_string()],
        output_shape: None,
    };
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        sql,
        Some("event_date"),
        &[silver_fact_edge(), edge2],
        &[],
        &[dim_source_with_unique_key()],
        &SourceReferentialIntegrity::new(),
    );
    for cell in &plan.cells {
        assert_eq!(
            cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
            Some(false),
            "every model edge is closed, but the unproven external inner join against 'dim' \
             must fail the shared AND to Open on cell {:?}, got {:?}",
            cell.trigger,
            cell.skeleton_source_closure
        );
    }
}

#[test]
fn model_edge_join_context_carries_source_unique_keys() {
    // No declared `unique_key` on the output; the proven grain
    // (`fact.customer_id`) is only trusted when the walk's fan-out check
    // sees the joined `dim` source's own declared unique key — otherwise
    // the join against `dim` is untrusted (fail-closed `OneToMany`) and no
    // grain is proven at all.
    let sql = "SELECT fact.customer_id, SUM(dim.amount) AS total \
        FROM smelt.silver.fact fact \
        JOIN smelt.sources.dim dim ON fact.dim_id = dim.id \
        GROUP BY fact.customer_id";

    let mut plan_without_key = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan_without_key,
        sql,
        Some("event_date"),
        &[silver_fact_edge()],
        &[],
        &[SourceFacts {
            name: "dim".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        }],
        &SourceReferentialIntegrity::new(),
    );
    assert_eq!(
        plan_without_key.cells[0].row_identity.identity,
        RowIdentity::WholeRow,
        "with no declared unique_key on 'dim', the join must stay untrusted and no grain key \
         proven, got {:?}",
        plan_without_key.cells[0].row_identity
    );

    let mut plan_with_key = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan_with_key,
        sql,
        Some("event_date"),
        &[silver_fact_edge()],
        &[],
        &[dim_source_with_unique_key()],
        &SourceReferentialIntegrity::new(),
    );
    assert_eq!(
        plan_with_key.cells[0].row_identity.identity,
        RowIdentity::Key(vec!["customer_id".to_string()]),
        "with 'dim's declared unique_key in the shared JoinContext, the join must be provably \
         one-to-one and the GROUP BY key trusted, got {:?}",
        plan_with_key.cells[0].row_identity
    );
}
