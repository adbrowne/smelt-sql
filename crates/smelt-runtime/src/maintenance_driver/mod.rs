//! Windowed-keyed-maintenance driver — the mode-agnostic mechanism behind
//! `refresh: keyed`'s window-forward run shape.
//!
//! See `docs/specs/model_transforms.md` §Surface "Windowed-keyed-maintenance
//! driver" and §Semantics "Keyed `merge_into`". The driver is the reusable
//! **classify → step over driving partitions in temporal order → per-partition
//! pushdown → create-or-merge** loop; `keyed` is its first named
//! consumer (`WindowedKeyedRule` impl in `crate::cumulative`).
//!
//! Fail-closed (`model_transforms.md` §Constraints "Equivalence or refusal"):
//! the driver never merges an unsafe combiner approximately. A
//! [`WindowedKeyedRule`] that cannot vouch for every step's combiner refuses
//! the whole run before any backend call is made.
//!
//! ## Module map
//!
//! Every maintenance statement this driver executes is the output of a pure
//! emitter in `smelt-logical`'s maintenance layer — the driver resolves live
//! facts, feeds them to those emitters, and executes what comes back
//! (`CLAUDE.md` §"Maintenance-plan purity"; gate:
//! `cargo test -p smelt-runtime --test statement_parity`). The submodules
//! below are pure code organisation over that one mechanism:
//!
//! - [`driver`] — the windowed-keyed loop itself: driving-partition
//!   stepping, the [`WindowedKeyedRule`] seam, and `run_windowed_keyed_maintenance`.
//! - [`resolve`] — plan-cell → live-technique resolution (creation strategy,
//!   fold deferral, column-scoped and in-place-update cells, horizon widening).
//! - [`membership`] — membership-sensitive recompute cells and their
//!   staged-candidate execution.
//! - [`repair`] — per-group recompute / repair cells and the diff-patch leg.
//! - [`key_addressed`] — key-addressed model-edge cells.
//! - [`column_scoped`] — column-scoped merge execution and the changed-row /
//!   changed-key predicates it dispatches on.
//! - [`observed_delta`] — reading an upstream driving model's observed delta
//!   back off the backend.
//! - [`sidecar`] — fingerprint and repair-group sidecar diffing and refresh.
//! - [`delta_restriction`] — delete+insert with a delta restriction, and the
//!   live facts that admit it.

mod column_scoped;
mod delta_restriction;
mod driver;
mod key_addressed;
mod membership;
mod observed_delta;
mod repair;
mod resolve;
mod sidecar;

#[cfg(test)]
mod tests;

pub use column_scoped::*;
pub use delta_restriction::*;
pub use driver::*;
pub use key_addressed::*;
pub use membership::*;
pub use observed_delta::*;
pub use repair::*;
pub use resolve::*;
pub use sidecar::*;
