//! Input gathering for the maintenance layer: resolving a model's refs to the
//! source / model-edge / clamp facts that `smelt-logical`'s pure maintenance
//! derivation reads, plus the `maintenance_plan` Salsa wrappers around it.
//!
//! Per the maintenance-plan purity rule (`CLAUDE.md` §"Maintenance-plan
//! purity"), nothing here derives a plan: every function assembles inputs and
//! calls into `smelt-logical`, or reads an already-derived verdict back.
//!
//! Split into [`refs`] (ref → source/timeseries/model-source-facts
//! resolution), [`edges`] (upstream model-edge and output-delta derivation),
//! [`clamps`] (per-source bound observability), and [`plan`] (the
//! `maintenance_plan`/`maintenance_plan_report` Salsa wrappers that thread
//! all of the above into `smelt-logical`'s pure derivation).

mod clamps;
mod edges;
mod plan;
mod refs;

pub use clamps::model_source_clamps;
pub use edges::{model_edges_for, model_output_delta_for};
pub use plan::{maintenance_plan, maintenance_plan_report};
pub(crate) use refs::{ref_source_info, ref_timeseries_config};
