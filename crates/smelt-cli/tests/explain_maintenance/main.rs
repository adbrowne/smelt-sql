//! MP7 (`docs/plans/20260707-maintenance-plan-impl.md`): `smelt explain
//! <model>` — the maintenance-plan report (`incremental_models.md` §Surface
//! "CLI"). Covers the pure report-string builder directly (fast) plus one
//! full CLI-argument-parsing path (spawns the real `smelt` binary) so the
//! wiring itself is exercised.
#![allow(dead_code, unused_imports)]

mod support;

mod clamps_and_headline;
mod contract_points;
mod degenerate_and_cli;
mod delta_types;
mod docs_and_technique;
mod execution_postures;
mod refusals;
mod repair;
mod state_downgrade;
mod state_section;
