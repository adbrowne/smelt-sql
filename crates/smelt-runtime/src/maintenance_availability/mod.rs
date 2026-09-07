//! The single `smelt-runtime` derivation seam
//! (`docs/outcomes/20260904-state-residency/outcome.md` phase 5): every
//! runtime consumer of `smelt-db`'s `derive_model_maintenance_plan{,
//! _with_edges}` reads the result through [`derive_resolved`] /
//! [`derive_resolved_with_edges`] instead, which apply
//! [`smelt_logical::maintenance::availability::resolve_availability`]
//! before any caller sees a cell (`state.md` §"The degradation contract",
//! step 2). No other module in this crate may call the `smelt-db` functions
//! directly — enforced structurally by
//! `crates/smelt-runtime/tests/availability_seam.rs`'s
//! `every_runtime_derivation_goes_through_the_availability_seam`.
//!
//! [`derive_resolved`] and [`derive_resolved_with_edges`] each live in their
//! own sibling module — twin wrappers over `smelt-db`'s two derivation
//! entry points, kept apart only for file size, not for any difference in
//! what they do with the result.

mod derive_resolved;
mod derive_resolved_with_edges;

pub use derive_resolved::derive_resolved;
pub use derive_resolved_with_edges::derive_resolved_with_edges;

use smelt_core::config::Config;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateAvailability};

/// The availability a run's target actually has: the intersection of what
/// `dialect` can realise and what `config.state.warehouse_tables` permits
/// (`state.md` §"Opting out of warehouse bookkeeping"). Built once per
/// target, not per model — a run's dialect and `state.warehouse_tables`
/// never vary across the models it drives against the same target.
pub fn availability_for_run(dialect: SqlDialect, config: &Config) -> StateAvailability {
    StateAvailability::resolve(
        config.state.warehouse_tables,
        &realisable_state_structures(dialect),
    )
}
