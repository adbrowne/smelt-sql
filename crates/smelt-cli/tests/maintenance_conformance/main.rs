#![cfg(feature = "duckdb")]
//! Standing generative maintenance-conformance gate
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3;
//! `docs/specs/maintenance_plan.md` §"The equivalence invariant"). Drives
//! the append-only partition-grain `ModelRecipe` pool
//! (`smelt_maintenance_testkit::recipe`) end-to-end through the real
//! `smelt_runtime::execute_project` pipeline, asserting S-restricted
//! multiset equivalence (`smelt_maintenance_testkit::s_tracker`) after every
//! run step.
//!
//! Unlike the disposable `property_discovery` research probes this target
//! builds on the same `smelt-maintenance-testkit` dev-dependency but is a
//! STANDING gate: it runs on every `cargo test`, deterministic-seeded, at a
//! small default case count (`SMELT_CONFORMANCE_CASES` env override — see
//! `gate.rs`).

mod dags;
mod gate;
mod harness_self_check;
mod pinned;
mod probes;
mod registry;
