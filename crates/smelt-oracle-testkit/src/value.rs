//! Typed result cells and the cross-engine value comparator.
//!
//! The schema leg cannot see the failure class this exists for: `2 ^ 3` is
//! `INT64` on both BigQuery and DuckDB, and returns `1` on one and `8` on the
//! other. Only comparing *values* catches that.
//!
//! `DuckDbOracle::execute_query`'s `format!("{val:?}")` rendering is
//! deliberately not reused: `"HugeInt(42)"` and Spark's `"42"` are the same
//! value, and a string comparator would call them divergent.

use arrow::array::{
    Array, BooleanArray, Decimal128Array, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::util::display::{ArrayFormatter, FormatOptions};

/// One result cell, normalised so two engines' renderings are comparable.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Int(i128),
    Float(f64),
    Bool(bool),
    Text(String),
    /// Value is `unscaled / 10^scale`. Kept unnormalised so the comparator can
    /// decide whether a scale difference matters.
    Decimal {
        unscaled: i128,
        scale: u32,
    },
    /// ISO-8601 `YYYY-MM-DD`.
    Date(String),
    /// ISO-8601, normalised to UTC.
    Timestamp(String),
}

/// Executes a query and returns typed rows.
///
/// A sibling to `TypeOracle`, not a widening of it: DuckDB is the only engine
/// that had both, and keeping them separate lets a schema-only engine stay a
/// schema-only engine.
pub trait ValueOracle {
    fn execute_rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueMatch {
    Equal,
    Divergent { detail: String },
}

fn divergent(reference: &Cell, actual: &Cell) -> ValueMatch {
    ValueMatch::Divergent {
        detail: format!("reference {reference:?} vs actual {actual:?}"),
    }
}

/// Rescale two decimals to a common scale and compare unscaled values.
///
/// Returns `None` when the rescale would overflow `i128` — a 10^39-scale
/// difference is not a value we can compare, and silently saying "equal" would
/// be the wrong answer.
fn decimals_equal(a: (i128, u32), b: (i128, u32)) -> Option<bool> {
    let (au, asc) = a;
    let (bu, bsc) = b;
    let target = asc.max(bsc);
    let lift = |u: i128, s: u32| -> Option<i128> { u.checked_mul(10i128.checked_pow(target - s)?) };
    Some(lift(au, asc)? == lift(bu, bsc)?)
}

fn as_f64(cell: &Cell) -> Option<f64> {
    match cell {
        Cell::Float(f) => Some(*f),
        Cell::Int(i) => Some(*i as f64),
        Cell::Decimal { unscaled, scale } => Some(*unscaled as f64 / 10f64.powi(*scale as i32)),
        _ => None,
    }
}

fn floats_equal(a: f64, b: f64) -> bool {
    if a.is_nan() && b.is_nan() {
        return true;
    }
    if a.is_infinite() || b.is_infinite() {
        return a == b;
    }
    (a - b).abs() <= 1e-9 * 1.0f64.max(a.abs().max(b.abs()))
}

/// Compare a target engine's cell against DuckDB's, under typed rules: exact
/// for integers, strings and booleans; relative tolerance for floats;
/// scale-normalised for decimals; NULL equals NULL.
pub fn compare_cells(reference: &Cell, actual: &Cell) -> ValueMatch {
    let equal = match (reference, actual) {
        (Cell::Null, Cell::Null) => true,
        (Cell::Null, _) | (_, Cell::Null) => false,

        (Cell::Bool(a), Cell::Bool(b)) => a == b,
        (Cell::Text(a), Cell::Text(b)) => a == b,
        (Cell::Date(a), Cell::Date(b)) => a == b,
        (Cell::Timestamp(a), Cell::Timestamp(b)) => a == b,
        (Cell::Int(a), Cell::Int(b)) => a == b,

        (
            Cell::Decimal {
                unscaled: au,
                scale: asc,
            },
            Cell::Decimal {
                unscaled: bu,
                scale: bsc,
            },
        ) => decimals_equal((*au, *asc), (*bu, *bsc)).unwrap_or(false),

        // Engines disagree on integer width and on whether `SUM` returns a
        // decimal; that is a *type* divergence, already registered in
        // `divergences.rs`, not a value one.
        (Cell::Int(n), Cell::Decimal { unscaled, scale })
        | (Cell::Decimal { unscaled, scale }, Cell::Int(n)) => {
            decimals_equal((*n, 0), (*unscaled, *scale)).unwrap_or(false)
        }

        (Cell::Float(_), _) | (_, Cell::Float(_)) => match (as_f64(reference), as_f64(actual)) {
            (Some(a), Some(b)) => floats_equal(a, b),
            _ => false,
        },

        _ => false,
    };
    if equal {
        ValueMatch::Equal
    } else {
        divergent(reference, actual)
    }
}

/// Decode one Arrow cell into a [`Cell`].
///
/// Used by [`crate::DuckDbOracle`]'s `ValueOracle` impl. Temporal values go
/// through Arrow's own ISO-8601 formatter rather than a hand-rolled epoch
/// conversion, so DuckDB's rendering matches the ISO strings the Spark and
/// BigQuery legs report.
pub(crate) fn cell_from_arrow(array: &dyn Array, row: usize) -> Cell {
    use arrow::datatypes::DataType as A;

    if array.is_null(row) {
        return Cell::Null;
    }

    macro_rules! int {
        ($ty:ty) => {
            match array.as_any().downcast_ref::<$ty>() {
                Some(a) => Cell::Int(a.value(row) as i128),
                None => Cell::Text(format!("<undecodable {:?}>", array.data_type())),
            }
        };
    }

    match array.data_type() {
        A::Boolean => match array.as_any().downcast_ref::<BooleanArray>() {
            Some(a) => Cell::Bool(a.value(row)),
            None => Cell::Text("<undecodable boolean>".into()),
        },
        A::Int8 => int!(Int8Array),
        A::Int16 => int!(Int16Array),
        A::Int32 => int!(Int32Array),
        A::Int64 => int!(Int64Array),
        A::UInt8 => int!(UInt8Array),
        A::UInt16 => int!(UInt16Array),
        A::UInt32 => int!(UInt32Array),
        A::UInt64 => int!(UInt64Array),
        A::Float32 => match array.as_any().downcast_ref::<Float32Array>() {
            Some(a) => Cell::Float(a.value(row) as f64),
            None => Cell::Text("<undecodable float32>".into()),
        },
        A::Float64 => match array.as_any().downcast_ref::<Float64Array>() {
            Some(a) => Cell::Float(a.value(row)),
            None => Cell::Text("<undecodable float64>".into()),
        },
        A::Decimal128(_, scale) => match array.as_any().downcast_ref::<Decimal128Array>() {
            // A negative Arrow scale means the unscaled value is multiplied,
            // not divided. Rendering it through the formatter keeps the value
            // right rather than silently flipping the sign of the exponent.
            Some(a) if *scale >= 0 => Cell::Decimal {
                unscaled: a.value(row),
                scale: *scale as u32,
            },
            _ => formatted(array, row),
        },
        A::Utf8 | A::LargeUtf8 | A::Utf8View => formatted_text(array, row),
        A::Date32 | A::Date64 => match formatted(array, row) {
            Cell::Text(t) => Cell::Date(t),
            other => other,
        },
        A::Timestamp(_, _) => match formatted(array, row) {
            Cell::Text(t) => Cell::Timestamp(t),
            other => other,
        },
        // Everything else (lists, structs, intervals, blobs) compares as its
        // canonical rendering. A structural comparator for those is not what
        // the emission audit needs, and pretending otherwise would hide the
        // gap.
        _ => formatted(array, row),
    }
}

fn formatted(array: &dyn Array, row: usize) -> Cell {
    match ArrayFormatter::try_new(array, &FormatOptions::default()) {
        Ok(f) => Cell::Text(f.value(row).to_string()),
        Err(e) => Cell::Text(format!("<unformattable: {e}>")),
    }
}

fn formatted_text(array: &dyn Array, row: usize) -> Cell {
    formatted(array, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_equals_null_and_nothing_else() {
        assert_eq!(compare_cells(&Cell::Null, &Cell::Null), ValueMatch::Equal);
        assert!(matches!(
            compare_cells(&Cell::Null, &Cell::Int(0)),
            ValueMatch::Divergent { .. }
        ));
        assert!(matches!(
            compare_cells(&Cell::Int(0), &Cell::Null),
            ValueMatch::Divergent { .. }
        ));
    }

    #[test]
    fn decimals_compare_on_value_not_on_scale() {
        let a = Cell::Decimal {
            unscaled: 150,
            scale: 2,
        }; // 1.50
        let b = Cell::Decimal {
            unscaled: 1500,
            scale: 3,
        }; // 1.500
        assert_eq!(compare_cells(&a, &b), ValueMatch::Equal);
        let c = Cell::Decimal {
            unscaled: 151,
            scale: 2,
        };
        assert!(matches!(
            compare_cells(&a, &c),
            ValueMatch::Divergent { .. }
        ));
    }

    #[test]
    fn an_integer_matches_a_scale_zero_decimal() {
        // DuckDB returns SUM(INTEGER) as Decimal(38,0); Spark returns BIGINT.
        assert_eq!(
            compare_cells(
                &Cell::Int(42),
                &Cell::Decimal {
                    unscaled: 42,
                    scale: 0
                }
            ),
            ValueMatch::Equal
        );
        // …and in the other direction.
        assert_eq!(
            compare_cells(
                &Cell::Decimal {
                    unscaled: 4200,
                    scale: 2
                },
                &Cell::Int(42)
            ),
            ValueMatch::Equal
        );
    }

    #[test]
    fn floats_compare_under_relative_tolerance() {
        assert_eq!(
            compare_cells(&Cell::Float(1.0), &Cell::Float(1.0 + 1e-12)),
            ValueMatch::Equal
        );
        assert!(matches!(
            compare_cells(&Cell::Float(1.0), &Cell::Float(1.001)),
            ValueMatch::Divergent { .. }
        ));
        assert_eq!(
            compare_cells(&Cell::Float(f64::NAN), &Cell::Float(f64::NAN)),
            ValueMatch::Equal
        );
        assert_eq!(
            compare_cells(&Cell::Float(f64::INFINITY), &Cell::Float(f64::INFINITY)),
            ValueMatch::Equal
        );
        assert!(matches!(
            compare_cells(&Cell::Float(f64::INFINITY), &Cell::Float(f64::NEG_INFINITY)),
            ValueMatch::Divergent { .. }
        ));
    }

    /// A float compares against the integer and decimal renderings of the same
    /// number: DuckDB's `POWER` returns DOUBLE where GoogleSQL may return
    /// FLOAT64 rendered as an integer-valued literal.
    #[test]
    fn a_float_compares_against_an_integer_of_the_same_value() {
        assert_eq!(
            compare_cells(&Cell::Float(8.0), &Cell::Int(8)),
            ValueMatch::Equal
        );
        assert!(matches!(
            compare_cells(&Cell::Float(8.0), &Cell::Int(1)),
            ValueMatch::Divergent { .. }
        ));
    }

    #[test]
    fn the_xor_case_is_caught() {
        // 2 ^ 3: power says 8, bitwise XOR says 1. Both are INT64 on BigQuery
        // and Spark, so the schema leg cannot see this. This is the whole point.
        assert!(matches!(
            compare_cells(&Cell::Int(8), &Cell::Int(1)),
            ValueMatch::Divergent { .. }
        ));
    }

    /// Mismatched families are divergent rather than silently coerced through
    /// a string rendering.
    #[test]
    fn a_text_cell_never_matches_a_numeric_one() {
        assert!(matches!(
            compare_cells(&Cell::Text("8".into()), &Cell::Int(8)),
            ValueMatch::Divergent { .. }
        ));
        assert!(matches!(
            compare_cells(&Cell::Bool(true), &Cell::Int(1)),
            ValueMatch::Divergent { .. }
        ));
    }

    /// A rescale that would overflow `i128` reports divergence rather than
    /// wrapping or claiming equality.
    #[test]
    fn an_unrepresentable_rescale_is_divergent_not_equal() {
        let a = Cell::Decimal {
            unscaled: 1,
            scale: 0,
        };
        let b = Cell::Decimal {
            unscaled: 1,
            scale: 38,
        };
        assert!(matches!(
            compare_cells(&a, &b),
            ValueMatch::Divergent { .. }
        ));
    }
}
