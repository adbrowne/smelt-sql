//! The contract lattice (`docs/specs/incremental_models.md` §"The contract
//! lattice"): a declared relaxation of the equivalence invariant is
//! admissible only as a complete single-owner triple — declaration schema,
//! pure oracle transform, and probe emitter — defined here, in
//! `smelt-logical`, never ad hoc by a caller
//! (`docs/outcomes/20260809-contract-lattice-v1/outcome.md`).
//!
//! The declaration *schema* (`smelt_core::config::ContractConfig`) lives one
//! layer down, in `smelt-core`, because `ModelMetadata` must deserialize it
//! and `smelt-core` sits below `smelt-logical` — the single-owner rule binds
//! the *semantics* (validation, oracle transform, probe emitter), not the
//! struct's crate.
//!
//! Landing status per lattice point:
//! - `frozen_horizon`: the complete triple has landed — grain-admissibility
//!   validation, the write-range clamp, and the late-arrival probe emitter
//!   all live in the `frozen_horizon` module.
//! - `deferral`: the complete triple has landed — clock-admissibility
//!   validation, the lag oracle, and the deferral-exceeded probe comparison
//!   all live in the `deferral` module. The two capabilities the point
//!   licenses, run skipping and work subsumption, have also landed —
//!   `deferral::run_license`, `deferral::pending_window`, and
//!   `deferral::subsumption` single-own the licensing decisions;
//!   `smelt-runtime`'s scheduler is a thin builder over them
//!   (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 5).
//! - `retain_departed`: the complete triple has landed — posture/
//!   tombstone-column validation, the departed-key quotient oracle, and the
//!   reconcile-anti-join probe emitter live in the `retain_departed`
//!   module; the runtime write-path seam (`retain_departed::
//!   reconcile_disposition`) resolves a declaration to the default point's
//!   anti-join delete leg (`smelt-logical`'s `emit_departed_key_delete`) or
//!   this point's probe dispatch, both driven from `smelt-runtime`'s
//!   `execute_snapshot_reconcile`.
//!
//! Split into [`grain_label`] (the refusal-message grain label),
//! [`effective`] (per-cell effective-contract resolution and its rendered
//! JSON view), and [`point`] (the `ContractPoint` oracle dispatch enum).

pub mod deferral;
mod effective;
pub mod frozen_horizon;
mod grain_label;
mod point;
pub mod retain_departed;

pub use effective::{
    effective_contract, ContractPointView, DeferralOrigin, EffectiveContract, EffectiveDeferral,
};
pub use grain_label::GrainLabel;
pub use point::{
    oracle_obligation, required_state_structure, restrict_run_window, settled_cutoff,
    ContractPoint, OracleObligation,
};
