//! Dev-only cross-engine oracle transport.
//!
//! Promoted out of `smelt-db/tests/prop_helpers/` so more than one crate's test
//! tree can probe a live engine. Derived test-support (dev-dependency of some
//! crate, regular dependency of none, no binary target), so it sits outside the
//! `unwrap`/`expect` ratchet's production set and must have no row in
//! `.claude/hardening-baseline.txt`.
//!
//! What lives here is the **transport plus the comparison primitives**: the
//! three oracles, the Arrow map, the error classifier, and `compare_types`.
//! `check_types_against_oracle` stayed in `smelt-db`, because it depends on
//! that crate's type inference and its SQL generators.

mod arrow_mapping;
mod bigquery_oracle;
mod duckdb_oracle;
mod error_class;
mod spark_oracle;
mod type_comparison;
mod value;

pub use arrow_mapping::arrow_to_smelt;
pub use bigquery_oracle::{BigQueryOracle, BqField};
pub use duckdb_oracle::{DuckDbOracle, TypeOracle};
pub use error_class::{classify_oracle_error, OracleErrorKind};
pub use spark_oracle::SparkOracle;
pub use type_comparison::{compare_types, TypeMatch};
pub use value::{compare_cells, Cell, ValueMatch, ValueOracle};
