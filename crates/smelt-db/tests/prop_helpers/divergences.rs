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
        // Names the single blanket compatibility rule in `type_comparison.rs`.
        // smelt models Text/Varchar(n)/Char(n) as one logical string type, so a
        // length/name difference from the backend is a designed leniency, not an
        // inference bug. `compare_types` returns Compatible for the string family
        // and cites this entry; it never reaches `find_divergence`, but the entry
        // exists so the leniency is named and greppable per the strictness rule.
        TypeDivergence {
            id: "text_varchar_compat",
            description: "Text/Varchar/Char — smelt has one logical string type; backends \
                distinguish VARCHAR(n)/CHAR(n)/TEXT. Authorises the string-family \
                Compatible verdict in type_comparison.rs.",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            status: DivergenceStatus::ByDesign,
        },
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
        // Decimal arithmetic model: smelt applies the portable, Spark-aligned
        // decimal growth formulas (spec §15) — multiplication p'=p1+p2+1,
        // s'=s1+s2; addition/subtraction/modulo p'=max(p1-s1,p2-s2)+max(s1,s2)+1;
        // ROUND keeps the input scale. DuckDB uses its native, physically-clamped
        // decimal arithmetic — multiplication clamps the result precision to the
        // storage-type boundary (BIGINT-backed DECIMAL, max 18 digits), ROUP(x)
        // reduces scale to 0, IFNULL widens precision to hold an integer literal,
        // and these differences propagate through nested arithmetic. On *raw*
        // (un-cast) SQL the two decimal models therefore disagree on precision
        // and/or scale across an open-ended family of expressions, so this is a
        // single named class divergence rather than one entry per (p,s) pair.
        //
        // Both operands are wildcards (`Decimal { precision: 0, scale: 0 }`): the
        // entry matches any Decimal-vs-Decimal pair. This is admissible — and not
        // a return to the removed blanket `is_decimal_compat` rule — because the
        // *exact* decimal correctness of smelt's inference is verified separately
        // and strictly by `tests/proptests/type_conformance_tests.rs`, which
        // cast-wraps smelt's inferred types and asserts DuckDB reproduces them
        // with zero divergence. A smelt-vs-DuckDB decimal difference on raw SQL is
        // expected; a cast-wrapped difference is a bug the conformance test fails
        // on. Verified DuckDB behaviours: `CAST(99.99 AS DECIMAL(10,2)) *
        // CAST(99.99 AS DECIMAL(10,2))` → DECIMAL(18,4) (smelt DECIMAL(21,4));
        // `ROUND(CAST(99.99 AS DECIMAL(10,2)))` → DECIMAL(10,0) (smelt (10,2));
        // `IFNULL(CAST(99.99 AS DECIMAL(10,2)), 0)` → DECIMAL(12,2) (smelt (10,2)).
        TypeDivergence {
            id: "decimal_arithmetic_model",
            description: "smelt uses Spark-aligned decimal growth (spec §15); DuckDB uses \
                physically-clamped native decimal arithmetic. On raw SQL the two disagree on \
                Decimal precision/scale across multiplication, ROUND, IFNULL, and nested \
                arithmetic. Exact decimal correctness is verified by the cast-wrap conformance \
                oracle (type_conformance_tests.rs); this entry tolerates the raw-SQL \
                Decimal-vs-Decimal difference only.",
            // Wildcard: matches any smelt Decimal.
            smelt_type: DataType::Decimal {
                precision: 0,
                scale: 0,
            },
            // Wildcard: matches any DuckDB Decimal.
            duckdb_type: Some(DataType::Decimal {
                precision: 0,
                scale: 0,
            }),
            // Spark uses the same Spark-aligned formulas as smelt, so no divergence
            // there; a Spark Decimal difference should still surface.
            spark_type: None,
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
        types_match(&d.smelt_type, smelt) && {
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
    fn decimal_arithmetic_model_matches_any_decimal_pair_duckdb() {
        let divs = known_divergences();
        // Multiplication precision growth.
        let found = find_divergence(
            &DataType::Decimal {
                precision: 21,
                scale: 4,
            },
            &DataType::Decimal {
                precision: 18,
                scale: 4,
            },
            "duckdb",
            &divs,
        );
        assert_eq!(found.unwrap().id, "decimal_arithmetic_model");
        // ROUND scale reduction (different scale) is also covered.
        let found2 = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Decimal {
                precision: 10,
                scale: 0,
            },
            "duckdb",
            &divs,
        );
        assert_eq!(found2.unwrap().id, "decimal_arithmetic_model");
    }

    #[test]
    fn decimal_wildcard_does_not_match_non_decimal_smelt() {
        let divs = known_divergences();
        // smelt Double vs DuckDB Decimal must NOT be absorbed by the decimal
        // wildcard (smelt side is not a Decimal).
        let found = find_divergence(
            &DataType::Double,
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            "duckdb",
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
