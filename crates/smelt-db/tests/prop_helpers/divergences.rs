//! Known type divergences between smelt inference and actual databases.
//!
//! Each divergence records what smelt infers vs what DuckDB and Spark actually
//! return, giving a unified view across backends.  When proptest finds a mismatch
//! that is already registered here, the test passes instead of failing.
//! Unknown mismatches still fail and print the full SQL for debugging.

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

/// A registered divergence between smelt and backends.
///
/// Each record shows what smelt infers and what each backend actually returns.
/// `None` means no divergence for that backend (smelt matches, or untested).
/// `Decimal { precision: 0, scale: 0 }` acts as a wildcard matching any Decimal.
#[derive(Debug)]
pub struct TypeDivergence {
    pub id: &'static str,
    pub description: &'static str,
    pub smelt_type: DataType,
    pub duckdb_type: Option<DataType>,
    pub spark_type: Option<DataType>,
    pub status: DivergenceStatus,
}

/// All known divergences.  Add new entries here when proptest surfaces expected mismatches.
pub fn known_divergences() -> Vec<TypeDivergence> {
    vec![
        TypeDivergence {
            id: "sum_integer",
            description: "SUM(INTEGER/BIGINT) — smelt infers BigInt, DuckDB returns Decimal(38,0) (HUGEINT via Arrow)",
            smelt_type: DataType::BigInt,
            duckdb_type: Some(DataType::Decimal {
                precision: 38,
                scale: 0,
            }),
            spark_type: None, // Spark also returns BigInt, matches smelt
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "string_concat",
            description: "|| operator — smelt infers Text, backends return Varchar/String",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "string_functions",
            description: "UPPER/LOWER/etc — smelt infers Text, backends return Varchar/String",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "ceil_floor_double",
            description: "CEIL/FLOOR(DOUBLE) — smelt returns Double (matches DuckDB), Spark returns BigInt",
            smelt_type: DataType::Double,
            duckdb_type: None,
            spark_type: Some(DataType::BigInt),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "avg_decimal",
            description:
                "AVG(DECIMAL) — smelt infers Double (matches DuckDB), Spark returns Decimal (varying precision)",
            smelt_type: DataType::Double,
            duckdb_type: None,
            // Wildcard: Decimal(0,0) matches any Decimal precision/scale
            spark_type: Some(DataType::Decimal {
                precision: 0,
                scale: 0,
            }),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "sign_double",
            description:
                "SIGN(DOUBLE) — smelt infers SmallInt (matches DuckDB TINYINT), Spark returns Double",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "sign_integer",
            description:
                "SIGN(INTEGER) — smelt infers SmallInt (matches DuckDB TINYINT), Spark returns Integer",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Integer),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "sign_bigint",
            description:
                "SIGN(BIGINT) — smelt infers SmallInt (matches DuckDB TINYINT), Spark returns BigInt",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::BigInt),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "sign_decimal",
            description:
                "SIGN(DECIMAL) — smelt infers SmallInt (matches DuckDB TINYINT), Spark returns Decimal",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            // Wildcard: Decimal(0,0) matches any Decimal precision/scale
            spark_type: Some(DataType::Decimal {
                precision: 0,
                scale: 0,
            }),
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "cast_float_as_double",
            description:
                "CAST(x AS FLOAT) — smelt normalizes FLOAT to DOUBLE, DuckDB returns FLOAT (4-byte)",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::Float),
            spark_type: None,
            status: DivergenceStatus::ByDesign,
        },
        TypeDivergence {
            id: "date_minus_date",
            description: "DATE - DATE — smelt infers Interval (Spark-aligned, the portable \
                temporal difference); DuckDB returns BIGINT (a plain day count). Spark also \
                returns an interval, matching smelt.",
            smelt_type: DataType::Interval,
            duckdb_type: Some(DataType::BigInt),
            spark_type: None, // Spark returns Interval, matches smelt
            status: DivergenceStatus::BackendSpecific,
        },
        TypeDivergence {
            id: "round_integer",
            description: "ROUND(INTEGER) — smelt's ROUND signature is Double→Double only; \
                integer inputs are upcast to Double before rounding, so smelt infers Double \
                while DuckDB preserves the integer type. Propagates to downstream arithmetic \
                on ROUND outputs (Double+Double in smelt vs Integer+Integer in DuckDB). \
                Fixing requires a polymorphic ROUND signature.",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::Integer),
            spark_type: None,
            status: DivergenceStatus::KnownBug,
        },
    ]
}

/// Check if a (smelt_type, actual_type) pair matches a known divergence for the given backend.
/// Returns the divergence if found.
pub fn find_divergence<'a>(
    smelt: &DataType,
    actual: &DataType,
    backend: &str,
    divergences: &'a [TypeDivergence],
) -> Option<&'a TypeDivergence> {
    divergences.iter().find(|d| {
        d.smelt_type == *smelt && {
            let expected = match backend {
                "duckdb" => d.duckdb_type.as_ref(),
                "spark" => d.spark_type.as_ref(),
                _ => None,
            };
            expected.is_some_and(|t| types_match(t, actual))
        }
    })
}

/// Check if a divergence's type pattern matches an actual type.
/// `Decimal { precision: 0, scale: 0 }` acts as a wildcard for any Decimal.
fn types_match(pattern: &DataType, actual: &DataType) -> bool {
    if pattern == actual {
        return true;
    }
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
    fn finds_sum_integer_divergence_duckdb() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 38,
                scale: 0,
            },
            "duckdb",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "sum_integer");
    }

    #[test]
    fn finds_ceil_floor_double_divergence_spark() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Double, &DataType::BigInt, "spark", &divs);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "ceil_floor_double");
    }

    #[test]
    fn backend_none_prevents_match() {
        let divs = known_divergences();
        // sum_integer has spark_type: None — should not match spark
        let found = find_divergence(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 38,
                scale: 0,
            },
            "spark",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn returns_none_for_unknown() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Boolean, &DataType::Date, "duckdb", &divs);
        assert!(found.is_none());
    }

    #[test]
    fn wildcard_decimal_matches_any_precision() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Double,
            &DataType::Decimal {
                precision: 14,
                scale: 6,
            },
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "avg_decimal");
    }
}
