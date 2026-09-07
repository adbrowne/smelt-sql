//! `execute_project` — the shared run pipeline.
//!
//! Composes the analysis layer (`smelt-db`, `smelt-core`, `smelt-planner`),
//! the compile layer (`smelt_runtime::compile`), and the selection layer
//! (`smelt_runtime::select`) into the full per-model execute loop. Both
//! `smelt-cli`'s `commands/run.rs` and `smelt-ui`'s `run_manager.rs`
//! consume this function via a `RunReporter` adapter.
//!
//! The pipeline owns the model-plan construction (batch dispatch per
//! `BatchSafety` shape), the per-model compile+execute loop (full refresh,
//! incremental batches, and keyed dispatch via `crate::cumulative`),
//! cancellation handling, manifest writes, and interval-store updates.
//!
//! The pipeline is split across sibling submodules of this directory;
//! this module re-exports the surface consumers depend on so that
//! `smelt_runtime::execute::{execute_project, BackendFactory, ...}` keeps
//! resolving exactly as it did when the pipeline lived in one file.

mod backend;
mod bootstrap;
mod key_addressed;
mod outcome;
mod plan;
mod project;
mod retry;
mod sink;
mod sources;
mod targets;
mod window;

pub use backend::{BackendFactory, BackendFuture};
pub use project::execute_project;
pub use retry::RetryPolicy;
pub use sources::{
    build_model_source_bounds, build_source_key_recurrence_map, build_source_timeseries_map,
    derive_batch_filtered_sql,
};

pub(crate) use retry::retry_backend_call;
pub(crate) use targets::sql_dialect_for_target;
