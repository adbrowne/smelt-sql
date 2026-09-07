//! `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`) must
//! stay in lockstep with the runtime classifier
//! (`smelt_logical::rules::cumulative::classify_order_monotone_column`) on
//! whether an order-monotone (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`) column
//! is admitted into a fold — otherwise `smelt explain`/LSP diagnostics
//! report a `KeyedFold` cell the runtime then refuses with
//! `KeyedUnknownCombiner` (a fail-loud discipline violation: a compile-time
//! false positive). Both layers admit an `ArgMax`/`ArgMin` column on hidden
//! `(v, o)` state (`docs/outcomes/20260809-rung2-state-shapes` row 5) — no
//! companion projection is required; only the wrong-arity shape refuses.
//!
//! Spec: `docs/specs/incremental_shapes.md` §"The column-family catalogue",
//! §"Statement emission (single owner)".

mod avg;
mod companion;
mod fallback;
mod once_write;
mod sum_unclocked;
