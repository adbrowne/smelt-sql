//! `smelt_logical::maintenance::refusal_code` agreement gate (ruling R2,
//! `docs/outcomes/20260905-property-diff/phases/02-plan.md` task 3;
//! fix round 1, F1/F2).
//!
//! `DiagnosticCode` lives in `smelt-db`, above `smelt-logical`
//! (`CLAUDE.md` §"Layered single-ownership"), so a profile refusal
//! (`smelt_logical::analysis::profile::ProfileRefusal`) can only carry the
//! diagnostic code's *name* as `Option<&'static str>`, not the enum value.
//! That trades a compile-time guarantee (an unrecognised `DiagnosticCode`
//! variant doesn't compile) for a runtime one — this test buys the
//! guarantee back in both directions:
//! - every `Some` name `refusal_code` returns must (a) name a real
//!   `DiagnosticCode` variant and (b) equal the code
//!   `smelt_db::queries::maintenance::diagnostic_for_refusal` — smelt-db's
//!   own, single-owned refusal → diagnostic mapping, also what
//!   `check_file_diagnostics` (`crates/smelt-db/src/lib.rs`) calls — emits
//!   for the same refusal shape, read from that function directly rather
//!   than from a `DiagnosticCode` typed into this test;
//! - every `None` `refusal_code` returns corresponds to a `Refusal` variant
//!   that has no `MaintenanceRefusal` counterpart at all (filtered to `None`
//!   before construction, `crates/smelt-db/src/queries/maintenance.rs`) —
//!   i.e. `smelt-db` really does raise no diagnostic for it today.

mod fixtures;
mod tests;
