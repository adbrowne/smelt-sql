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
//!   licenses (run skipping, work subsumption) are separate, later work
//!   (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 5).

pub mod deferral;
pub mod frozen_horizon;
