//! Phase 42 — Lowering helpers for the logical→physical translation.
//!
//! This module owns SQL-emission helpers that bridge a `LogicalNode`
//! subtree to backend-specific SQL text. Each helper is pure (no Salsa,
//! no I/O); inputs flow in via parameters so the helpers can be called
//! from any caller — Salsa-driven LSP, CLI batch compile, or planner
//! tests.

pub mod as_struct;

pub use as_struct::{as_struct_to_sql, backend_supports_struct_literal};
