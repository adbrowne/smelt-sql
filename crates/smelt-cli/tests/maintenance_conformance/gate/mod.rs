//! The standing proptest gate over the append-only partition-grain
//! `ModelRecipe` pool
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3),
//! plus the fact+mutable-dimension mixed pool (Phase 4).
//!
//! The gate is split by recipe family / step family; each submodule owns one
//! pool's staging, classification, oracle and its own `#[test]` entry points,
//! and the shared plumbing lives in [`support`]. Sibling test files in this
//! directory consume the staging/drive/oracle helpers re-exported below.

pub(crate) mod change_feed;
pub(crate) mod composed_pool;
pub(crate) mod composed_routes;
pub(crate) mod composed_support;
pub(crate) mod definition_change;
pub(crate) mod delta_restriction;
pub(crate) mod keyed_enriched;
pub(crate) mod keyed_oracle;
pub(crate) mod keyed_pool;
pub(crate) mod keyed_snapshot_reconcile;
pub(crate) mod keyed_support;
pub(crate) mod migrate_step;
pub(crate) mod mixed_pool;
pub(crate) mod once_write;
pub(crate) mod partition_pool;
pub(crate) mod schedule_enrichment;
pub(crate) mod support;
pub(crate) mod technique_agreement;
pub(crate) mod value_enriched;

pub(crate) use keyed_oracle::{classify_keyed, drive_keyed_and_assert};
pub(crate) use keyed_support::{insert_row_keyed, stage_keyed_recipe};
pub(crate) use mixed_pool::{classify_mixed, insert_fact_row, stage_mixed_recipe};
pub(crate) use partition_pool::{
    assert_equivalence, assert_equivalence_at_point, assert_equivalence_at_point_with_frontier,
    drive_and_assert, stage_recipe,
};
pub(crate) use support::snapshot_table_rows;
