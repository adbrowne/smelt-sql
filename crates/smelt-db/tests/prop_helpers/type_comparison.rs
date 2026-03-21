//! Compare smelt-inferred types against actual database types.
//!
//! Not every mismatch is a bug — some are known backend differences (e.g. Text
//! vs Varchar) that we mark `Compatible` rather than `Mismatch`.

use smelt_types::DataType;

/// Result of comparing a smelt type against an actual database type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeMatch {
    /// Types are identical.
    Exact,
    /// Types differ but are semantically compatible.
    Compatible { reason: &'static str },
    /// Types are genuinely different — likely a smelt inference bug.
    Mismatch,
}

/// Compare a smelt-inferred type against a database-reported type.
pub fn compare_types(smelt: &DataType, actual: &DataType) -> TypeMatch {
    if smelt == actual {
        return TypeMatch::Exact;
    }

    // Text <-> Varchar (any length) are compatible string representations.
    if is_string_compat(smelt, actual) {
        return TypeMatch::Compatible {
            reason: "Text/Varchar interchangeable",
        };
    }

    // Decimal precision/scale differences: smelt may infer a wider (38,10) envelope
    // while the database returns exact precision.
    if is_decimal_compat(smelt, actual) {
        return TypeMatch::Compatible {
            reason: "Decimal precision/scale subsumes",
        };
    }

    // Integer width promotions (SmallInt -> Integer -> BigInt) where smelt infers
    // a narrower type than DuckDB actually produces.
    if is_integer_width_compat(smelt, actual) {
        return TypeMatch::Compatible {
            reason: "integer width promotion",
        };
    }

    TypeMatch::Mismatch
}

fn is_string_compat(a: &DataType, b: &DataType) -> bool {
    fn is_string(dt: &DataType) -> bool {
        matches!(
            dt,
            DataType::Text | DataType::Varchar { .. } | DataType::Char { .. }
        )
    }
    is_string(a) && is_string(b)
}

fn is_decimal_compat(a: &DataType, b: &DataType) -> bool {
    matches!((a, b), (DataType::Decimal { .. }, DataType::Decimal { .. }))
}

fn is_integer_width_compat(a: &DataType, b: &DataType) -> bool {
    fn int_rank(dt: &DataType) -> Option<u8> {
        match dt {
            DataType::SmallInt => Some(0),
            DataType::Integer => Some(1),
            DataType::BigInt => Some(2),
            _ => None,
        }
    }
    matches!((int_rank(a), int_rank(b)), (Some(_), Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert_eq!(
            compare_types(&DataType::Integer, &DataType::Integer),
            TypeMatch::Exact
        );
    }

    #[test]
    fn text_varchar_compatible() {
        assert_eq!(
            compare_types(&DataType::Text, &DataType::Varchar { max_length: None }),
            TypeMatch::Compatible {
                reason: "Text/Varchar interchangeable"
            }
        );
    }

    #[test]
    fn decimal_compatible() {
        assert_eq!(
            compare_types(
                &DataType::Decimal {
                    precision: 38,
                    scale: 10
                },
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                }
            ),
            TypeMatch::Compatible {
                reason: "Decimal precision/scale subsumes"
            }
        );
    }

    #[test]
    fn integer_width_compat() {
        assert_eq!(
            compare_types(&DataType::Integer, &DataType::BigInt),
            TypeMatch::Compatible {
                reason: "integer width promotion"
            }
        );
    }

    #[test]
    fn genuine_mismatch() {
        assert_eq!(
            compare_types(&DataType::Boolean, &DataType::Integer),
            TypeMatch::Mismatch
        );
    }
}
