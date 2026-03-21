//! Known type divergences between smelt inference and actual databases.
//!
//! When proptest finds a mismatch that is already registered here, the test
//! passes with a warning instead of failing.  Unknown mismatches still fail
//! and print the full SQL for debugging.

use smelt_types::DataType;

/// Why this divergence exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// A bug in smelt's inference that we plan to fix.
    KnownBug,
    /// Intentional design choice in smelt.
    ByDesign,
    /// Database-specific behavior we can't fully model.
    BackendSpecific,
}

/// A registered divergence between smelt and a backend.
#[derive(Debug)]
pub struct TypeDivergence {
    pub id: &'static str,
    pub description: &'static str,
    pub smelt_type: DataType,
    pub actual_type: DataType,
    pub backend: &'static str,
    pub status: DivergenceStatus,
}

/// All known divergences.  Add new entries here when proptest surfaces expected mismatches.
pub fn known_divergences() -> Vec<TypeDivergence> {
    vec![
        TypeDivergence {
            id: "sum_integer_to_decimal",
            description: "SUM(INTEGER) — smelt infers Decimal(38,10), DuckDB returns BigInt (HUGEINT mapped to BigInt)",
            smelt_type: DataType::Decimal { precision: 38, scale: 10 },
            actual_type: DataType::BigInt,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "sum_double_to_decimal",
            description: "SUM(DOUBLE) — smelt infers Decimal(38,10), DuckDB returns Double",
            smelt_type: DataType::Decimal { precision: 38, scale: 10 },
            actual_type: DataType::Double,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "extract_double_vs_bigint",
            description: "EXTRACT(...) — smelt infers Double, DuckDB returns BigInt",
            smelt_type: DataType::Double,
            actual_type: DataType::BigInt,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "date_trunc_returns_timestamp",
            description: "DATE_TRUNC(...) — smelt infers Date, DuckDB returns Timestamp",
            smelt_type: DataType::Date,
            actual_type: DataType::Timestamp { with_timezone: false },
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "length_integer_vs_bigint",
            description: "LENGTH(...) — smelt infers Integer, DuckDB returns BigInt",
            smelt_type: DataType::Integer,
            actual_type: DataType::BigInt,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "string_concat_text_vs_varchar",
            description: "|| operator — smelt infers Text, DuckDB returns Varchar",
            smelt_type: DataType::Text,
            actual_type: DataType::Varchar { max_length: None },
            backend: "duckdb",
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "string_functions_text_vs_varchar",
            description: "UPPER/LOWER/etc — smelt infers Text, DuckDB returns Varchar",
            smelt_type: DataType::Text,
            actual_type: DataType::Varchar { max_length: None },
            backend: "duckdb",
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "ceil_floor_integer_to_double",
            description: "CEIL/FLOOR(INTEGER) — smelt preserves arg type, DuckDB returns Double",
            smelt_type: DataType::Integer,
            actual_type: DataType::Double,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "ceil_floor_bigint_to_double",
            description: "CEIL/FLOOR(BIGINT) — smelt preserves arg type, DuckDB returns Double",
            smelt_type: DataType::BigInt,
            actual_type: DataType::Double,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "ceil_floor_decimal_to_double",
            description: "CEIL/FLOOR(DECIMAL) — smelt preserves arg type, DuckDB returns Double",
            smelt_type: DataType::Decimal { precision: 10, scale: 2 },
            actual_type: DataType::Double,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "position_integer_vs_bigint",
            description: "STRPOS/POSITION — smelt infers Integer, DuckDB returns BigInt",
            smelt_type: DataType::Integer,
            actual_type: DataType::BigInt,
            backend: "duckdb",
            status: DivergenceStatus::KnownBug,
        },
        // ---- Spark divergences ----
        TypeDivergence {
            id: "spark_sum_integer_to_decimal",
            description: "SUM(INTEGER) — smelt infers Decimal(38,10), Spark returns BigInt",
            smelt_type: DataType::Decimal { precision: 38, scale: 10 },
            actual_type: DataType::BigInt,
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "spark_sum_double_to_decimal",
            description: "SUM(DOUBLE) — smelt infers Decimal(38,10), Spark returns Double",
            smelt_type: DataType::Decimal { precision: 38, scale: 10 },
            actual_type: DataType::Double,
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "spark_string_concat_text_vs_string",
            description: "|| operator — smelt infers Text, Spark returns Varchar (string)",
            smelt_type: DataType::Text,
            actual_type: DataType::Varchar { max_length: None },
            backend: "spark",
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "spark_string_functions_text_vs_string",
            description: "UPPER/LOWER/etc — smelt infers Text, Spark returns Varchar (string)",
            smelt_type: DataType::Text,
            actual_type: DataType::Varchar { max_length: None },
            backend: "spark",
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "spark_avg_decimal_to_double",
            description: "AVG(DECIMAL) — smelt infers Double, Spark returns Decimal (any precision)",
            smelt_type: DataType::Double,
            // precision: 0, scale: 0 is a wildcard — matches any Decimal
            actual_type: DataType::Decimal { precision: 0, scale: 0 },
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "spark_ceil_floor_double_to_bigint",
            description: "CEIL/FLOOR(DOUBLE) — smelt preserves arg type Double, Spark returns BigInt",
            smelt_type: DataType::Double,
            actual_type: DataType::BigInt,
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "spark_extract_double_vs_integer",
            description: "EXTRACT(...) — smelt infers Double, Spark returns Integer",
            smelt_type: DataType::Double,
            actual_type: DataType::Integer,
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
        TypeDivergence {
            id: "spark_date_trunc_returns_timestamp",
            description: "DATE_TRUNC(...) — smelt infers Date, Spark returns Timestamp",
            smelt_type: DataType::Date,
            actual_type: DataType::Timestamp { with_timezone: false },
            backend: "spark",
            status: DivergenceStatus::KnownBug,
        },
    ]
}

/// Check if a (smelt_type, actual_type) pair matches a known divergence for the given backend.
/// Returns the divergence if found.
///
/// Supports wildcard matching: `Decimal { precision: 0, scale: 0 }` in a divergence
/// matches any `Decimal` actual type regardless of precision/scale.
pub fn find_divergence<'a>(
    smelt: &DataType,
    actual: &DataType,
    backend: &str,
    divergences: &'a [TypeDivergence],
) -> Option<&'a TypeDivergence> {
    divergences.iter().find(|d| {
        d.smelt_type == *smelt && types_match(&d.actual_type, actual) && d.backend == backend
    })
}

/// Check if a divergence's type pattern matches an actual type.
/// `Decimal { precision: 0, scale: 0 }` acts as a wildcard for any Decimal.
fn types_match(pattern: &DataType, actual: &DataType) -> bool {
    if pattern == actual {
        return true;
    }
    // Wildcard: Decimal(0,0) matches any Decimal
    matches!(
        (pattern, actual),
        (
            DataType::Decimal {
                precision: 0,
                scale: 0
            },
            DataType::Decimal { .. }
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_sum_divergence_duckdb() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            &DataType::BigInt,
            "duckdb",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "sum_integer_to_decimal");
    }

    #[test]
    fn finds_known_sum_divergence_spark() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            &DataType::BigInt,
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "spark_sum_integer_to_decimal");
    }

    #[test]
    fn backend_filter_prevents_cross_match() {
        let divs = known_divergences();
        // DuckDB's length divergence (Integer vs BigInt) shouldn't match spark backend
        let found = find_divergence(&DataType::Integer, &DataType::BigInt, "spark", &divs);
        assert!(found.is_none());
    }

    #[test]
    fn returns_none_for_unknown() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Boolean, &DataType::Date, "duckdb", &divs);
        assert!(found.is_none());
    }
}
