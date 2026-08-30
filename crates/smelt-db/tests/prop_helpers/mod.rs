//! Helper modules for property-based type inference tests.
//!
//! These tests live in `smelt-db` (not `smelt-types`) because:
//! - Type inference code (`infer_expression_type`, `TypeContext`, etc.) lives in `smelt-db`
//! - `smelt-db` already depends on `smelt-parser` and `smelt-types`
//! - Placing tests in `smelt-types` would create a circular dependency
//!
//! The strategy: generate random SQL expressions with known types via CTEs, run them
//! against DuckDB to get actual types, and compare against smelt's type inference.
//! Mismatches are either bugs (to fix), known divergences (registered in `divergences.rs`),
//! or compatible type differences (Text vs Varchar, Decimal precision differences).

pub mod divergences;
pub mod generators;
pub mod known_unknowns;
pub mod monotonicity_gen;
pub mod null_data;
pub mod oracle_check;

// The oracle transport (the three oracles, the Arrow map, the error
// classifier, and `compare_types`) lives in `smelt-oracle-testkit`. Import it
// directly — there is deliberately no re-export shim here.
