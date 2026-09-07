//! Maintenance plan — v0 tracer bullet.
//!
//! A model's incremental maintenance as a plan indexed by
//! `(output-column-group × trigger)`, each cell landing in a corner of the
//! read-scope × write-scope 2×2. This is the datatype proposed by
//! `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` (§3,
//! §5) placed per `08-code-placement.md` §2.1, built here as a **tracer
//! bullet**: enough machinery to derive plans and emit maintenance SQL for
//! the catalogue's key examples (EX-02, EX-07, EX-13, EX-24, EX-36–40 of
//! `07-example-catalogue.md`) and prove equivalence against a full refresh.
//!
//! Honest v0 boundaries (see `09-spec-readiness.md` §2):
//! - Column groups (`ColumnGroup`) and skeleton columns
//!   (`OutputSpec::skeleton_columns`) may now be **derived** —
//!   [`grouping::derive_column_groups`] and [`skeleton::skeleton_columns`] —
//!   or still hand-supplied by a caller that needs a shape outside their v0
//!   scope (a CTE/set-operation-composed model); `derive_maintenance_plan`
//!   itself is agnostic to which and keeps taking `ColumnGroup`/
//!   `skeleton_columns` as plain data.
//! - Scan bounds are derived (`analysis::source_bounds`), combiner algebra is
//!   derived (`analysis::discriminants`), additive-only column adds are
//!   proven (`analysis::model_diff`) — the derivations that exist are
//!   consumed, the ones that don't are inputs.
//!
//! Nothing here is wired into diagnostics, planning, or execution; the module
//! is pure data + pure functions (Salsa-purity compatible by construction).

pub mod availability;
pub mod choice;
pub mod derive;
pub mod diff_patch;
pub mod edge_type;
pub mod emit;
pub mod granularity;
pub mod grouping;
pub mod locality;
pub mod probe_cadence;
pub mod propagate;
pub mod repair;
pub mod skeleton;
pub mod succession;

mod plan;
mod refusal;
mod types;
mod write_pattern;

pub use plan::{
    column_comparability_with_contract, identity_not_derivable_plan, locality_refused_plan,
    recurrence_mismatch_plan, succession_refused_plan, unsupported_grain_plan, KeyLocality,
    MaintenancePlan,
};
pub use refusal::{refusal_code, Refusal};
pub use types::{
    cell_trigger_address, ColumnGroup, Corner, Grain, KeyDiscovery, KeyScope, MutationProfile,
    OutputSpec, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, ScanClamp, SourceFacts,
    Technique, Trigger,
};
pub use write_pattern::{
    admissible_write_patterns, cell_equivalence_proof, lookup_write_pattern, pattern_admissible,
    pattern_capability_available, pattern_facts_satisfied, resolve_write_pin,
    BackendWriteCapabilities, ContractFact, OutputContractFacts, WriteCapability, WritePattern,
    WritePinRefusal, WriteSelection,
};

pub use crate::analysis::fingerprint::Projection as FingerprintProjection;
pub use crate::analysis::skeleton_closure::{RowPreservation, SkeletonSourceClosure};
pub use probe_cadence::{should_dispatch, ProbeDispatch, SkipReason};
