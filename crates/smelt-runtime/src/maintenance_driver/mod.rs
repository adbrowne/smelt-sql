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
//! Every maintenance statement this driver executes is the output of a pure
//! emitter in `smelt-logical`'s maintenance layer — the driver resolves live
//! facts, feeds them to those emitters, and executes what comes back
//! (`CLAUDE.md` §"Maintenance-plan purity"; gate:
//! `cargo test -p smelt-runtime --test statement_parity`). The submodules
//! below are pure code organisation over that one mechanism; see this
//! crate's `CLAUDE.md` §"Windowed-keyed-maintenance driver module map" for
//! what each one owns.

mod column_scoped;
mod delta_restriction;
mod driver;
mod key_addressed;
mod membership;
mod observed_delta;
mod repair;
mod resolve;
mod sidecar;
pub mod succession;

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
pub use succession::{
    build_succession_source_refs, execute_succession_maintenance, resolve_live_succession_cell,
    SuccessionCell,
};
