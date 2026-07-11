//! Compare smelt-inferred types against actual database types.
//!
//! The comparator is **strict by default**: integer widths (SmallInt vs Integer
//! vs BigInt) and Decimal precision/scale must match *exactly*. There is exactly
//! one blanket compatibility rule — the string family (Text/Varchar/Char) — and
//! it exists only because it is registered as a named `ByDesign` divergence
//! (`text_varchar_compat` in `divergences.rs`): smelt models all fixed/variable
//! character types as one logical string type, so a length/name difference from
//! the backend is a designed leniency, not an inference bug.
//!
//! Every *other* tolerated difference (integer width, decimal precision/scale,
//! FLOAT/DOUBLE normalisation, aggregate return widening, …) is NOT absorbed
//! here. It surfaces as `Mismatch` and must be routed through a named entry in
//! `divergences.rs`, so each one is individually documented and greppable.

use smelt_types::DataType;

/// Result of comparing a smelt type against an actual database type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeMatch {
    /// Types are identical.
    Exact,
    /// Types differ but are semantically compatible under a named `ByDesign`
    /// divergence. The `reason` names the divergence that authorises it.
    Compatible { reason: &'static str },
    /// Types are genuinely different — either a smelt inference bug or a
    /// difference that must be registered in `divergences.rs`.
    Mismatch,
}

/// Compare a smelt-inferred type against a database-reported type.
///
/// Strict: the only non-`Exact` result that is treated as `Compatible` is the
/// string family, backed by the `text_varchar_compat` `ByDesign` divergence.
/// Integer-width and Decimal-precision differences are `Mismatch` and must be
/// registered as named divergences to pass the property test.
pub fn compare_types(smelt: &DataType, actual: &DataType) -> TypeMatch {
    if smelt == actual {
        return TypeMatch::Exact;
    }

    // The single blanket compatibility rule, authorised by the named
    // `text_varchar_compat` ByDesign divergence: smelt has one logical string
    // type; backends distinguish VARCHAR(n)/CHAR(n)/TEXT.
    if is_string_compat(smelt, actual) {
        return TypeMatch::Compatible {
            reason: "text_varchar_compat: string family interchangeable (ByDesign)",
        };
    }

    // Structural recursion for arrays: `ARRAY_AGG(string_expr)` yields
    // Array(Text) in smelt vs Array(Varchar) from DuckDB — the same string-family
    // leniency one level down. Element-wise comparison keeps the verdict aligned
    // with the scalar case (and surfaces genuine element-type mismatches, e.g.
    // Array(Integer) vs Array(BigInt), as Mismatch).
    if let (DataType::Array(a), DataType::Array(b)) = (smelt, actual) {
        return compare_types(a, b);
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
        // String-family leniency is retained — it is the one named ByDesign
        // blanket rule (`text_varchar_compat`).
        assert!(matches!(
            compare_types(&DataType::Text, &DataType::Varchar { max_length: None }),
            TypeMatch::Compatible { .. }
        ));
    }

    #[test]
    fn integer_width_is_mismatch() {
        // Strict widths: Integer vs BigInt is NOT compatible anymore. It must be
        // registered as a named divergence to pass the property test.
        assert_eq!(
            compare_types(&DataType::Integer, &DataType::BigInt),
            TypeMatch::Mismatch
        );
        assert_eq!(
            compare_types(&DataType::SmallInt, &DataType::Integer),
            TypeMatch::Mismatch
        );
    }

    #[test]
    fn decimal_precision_is_mismatch() {
        // Strict precision/scale: a wider smelt envelope vs an exact backend
        // precision is a Mismatch, not a blanket compatibility.
        assert_eq!(
            compare_types(
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                },
                &DataType::Decimal {
                    precision: 38,
                    scale: 2
                }
            ),
            TypeMatch::Mismatch
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
