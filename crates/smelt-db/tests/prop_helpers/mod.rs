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

pub mod arrow_mapping;
pub mod divergences;
pub mod duckdb_oracle;
pub mod generators;
pub mod known_unknowns;
pub mod monotonicity_gen;
pub mod null_data;
pub mod spark_oracle;
pub mod type_comparison;
