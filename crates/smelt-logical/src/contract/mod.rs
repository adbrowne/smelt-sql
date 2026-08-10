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
//! - `deferral`: unbuilt — declaration is refused fail-loud
//!   (`smelt_core::config::ContractConfig` is `deny_unknown_fields`) until
//!   it is wired.

pub mod frozen_horizon;
