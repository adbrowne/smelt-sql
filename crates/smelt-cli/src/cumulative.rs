//! Cumulative aggregate dispatch — see `docs/specs/cumulative_aggregate.md`.
//!
//! The implementation lives in `smelt-runtime::cumulative`; this re-export
//! keeps existing `smelt_cli::cumulative::*` callers compiling unchanged.
//! New callers should import from `smelt_runtime` directly.

pub use smelt_runtime::cumulative::{build_cumulative_merge_sql, execute_cumulative_aggregate};
