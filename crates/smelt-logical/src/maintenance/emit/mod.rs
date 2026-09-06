//! Physical maintenance SQL emission — the single author of every
//! maintenance statement a run executes
//! (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
//!
//! One emitter per [`Technique`](super::Technique), following the
//! physical-maintenance notation of
//! `docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`:
//! the partition predicate is carried on **both** the scan and the write
//! target wherever the op is region-scoped — a predicate stated only on one
//! side is a logical bound the storage layer cannot use
//! (`01-framework.md` §5).
//!
//! Emission is pure string construction over a caller-supplied SELECT body
//! (the model SQL with source refs resolved to physical table names); clamp
//! *injection into* the body is the runtime transformer's job
//! (`smelt-runtime/src/transformer.rs`) and is deliberately not duplicated
//! here — an emitter never adds a predicate the caller did not already fold
//! into the body it hands in, so the emitted text is exactly what a backend
//! executes, byte for byte.
//!
//! Backends *execute* the [`StatementGroup`]s these functions return; they
//! never author maintenance-statement text of their own
//! (`docs/specs/architecture.md` §"Constraints & Invariants" item 12).

mod bootstrap;
mod fingerprint;
mod merge;
mod probes;
mod projection;
mod recompute;
mod staged;
mod types;

pub use bootstrap::*;
pub use fingerprint::*;
pub use merge::*;
pub use probes::*;
pub use projection::*;
pub use recompute::*;
pub use staged::*;
pub use types::*;
