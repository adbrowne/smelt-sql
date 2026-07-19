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

    // Structural recursion for structs: field-exact by default (name and type
    // must agree per field), but with one named leniency mirroring the string
    // rule above — `row_constructor_field_naming` (ByDesign, in
    // `divergences.rs`): DuckDB's `ROW(...)` leaves fields anonymous (empty
    // name), while smelt names positional struct fields v1, v2, ... for
    // ergonomic dot-access. A struct field is compared as "compatible" when
    // the actual (backend) name is empty and the smelt name follows that
    // convention; any other name mismatch is a genuine Mismatch.
    if let (DataType::Struct(a), DataType::Struct(b)) = (smelt, actual) {
        return compare_struct_fields(a, b);
    }

    TypeMatch::Mismatch
}

fn compare_struct_fields(
    smelt_fields: &[(String, DataType)],
    actual_fields: &[(String, DataType)],
) -> TypeMatch {
    if smelt_fields.len() != actual_fields.len() {
        return TypeMatch::Mismatch;
    }

    let mut compat_reason: Option<&'static str> = None;
    for (i, ((s_name, s_ty), (a_name, a_ty))) in
        smelt_fields.iter().zip(actual_fields.iter()).enumerate()
    {
        if a_name.is_empty() && *s_name == format!("v{}", i + 1) {
            compat_reason = Some(
                "row_constructor_field_naming: smelt names positional struct fields \
                 v1, v2, ...; DuckDB's ROW(...) leaves them anonymous (ByDesign)",
            );
        } else if s_name != a_name {
            return TypeMatch::Mismatch;
        }

        match compare_types(s_ty, a_ty) {
            TypeMatch::Exact => {}
            TypeMatch::Compatible { reason } => compat_reason = Some(reason),
            TypeMatch::Mismatch => return TypeMatch::Mismatch,
        }
    }

    match compat_reason {
        Some(reason) => TypeMatch::Compatible { reason },
        None => TypeMatch::Exact,
    }
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

    #[test]
    fn struct_exact_match() {
        let s = DataType::Struct(vec![
            ("a".to_string(), DataType::Integer),
            ("b".to_string(), DataType::Boolean),
        ]);
        assert_eq!(compare_types(&s, &s), TypeMatch::Exact);
    }

    #[test]
    fn struct_field_type_mismatch_is_mismatch() {
        let smelt = DataType::Struct(vec![("a".to_string(), DataType::Integer)]);
        let actual = DataType::Struct(vec![("a".to_string(), DataType::BigInt)]);
        assert_eq!(compare_types(&smelt, &actual), TypeMatch::Mismatch);
    }

    #[test]
    fn struct_field_name_mismatch_is_mismatch() {
        let smelt = DataType::Struct(vec![("a".to_string(), DataType::Integer)]);
        let actual = DataType::Struct(vec![("z".to_string(), DataType::Integer)]);
        assert_eq!(compare_types(&smelt, &actual), TypeMatch::Mismatch);
    }

    #[test]
    fn struct_field_count_mismatch_is_mismatch() {
        let smelt = DataType::Struct(vec![
            ("a".to_string(), DataType::Integer),
            ("b".to_string(), DataType::Boolean),
        ]);
        let actual = DataType::Struct(vec![("a".to_string(), DataType::Integer)]);
        assert_eq!(compare_types(&smelt, &actual), TypeMatch::Mismatch);
    }

    #[test]
    fn struct_row_constructor_naming_is_compatible() {
        // smelt names positional ROW(...) fields v1, v2; DuckDB leaves them
        // anonymous ("") — the `row_constructor_field_naming` ByDesign leniency.
        let smelt = DataType::Struct(vec![
            ("v1".to_string(), DataType::Integer),
            ("v2".to_string(), DataType::Boolean),
        ]);
        let actual = DataType::Struct(vec![
            (String::new(), DataType::Integer),
            (String::new(), DataType::Boolean),
        ]);
        assert!(matches!(
            compare_types(&smelt, &actual),
            TypeMatch::Compatible { .. }
        ));
    }

    #[test]
    fn struct_nested_string_field_is_compatible() {
        // Nested recursion also picks up the text/varchar leniency per field.
        let smelt = DataType::Struct(vec![("a".to_string(), DataType::Text)]);
        let actual = DataType::Struct(vec![(
            "a".to_string(),
            DataType::Varchar { max_length: None },
        )]);
        assert!(matches!(
            compare_types(&smelt, &actual),
            TypeMatch::Compatible { .. }
        ));
    }
}
