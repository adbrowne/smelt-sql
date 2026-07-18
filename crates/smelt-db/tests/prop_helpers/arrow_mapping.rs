//! Maps Arrow DataType to smelt DataType.
//!
//! DuckDB returns query results as Arrow RecordBatches; this module translates
//! the Arrow schema types into smelt's canonical `DataType` so we can compare
//! what DuckDB actually produced against what smelt's type inference predicted.

use arrow::datatypes::DataType as ArrowType;
use smelt_types::DataType;

/// Convert an Arrow DataType to the closest smelt DataType.
pub fn arrow_to_smelt(arrow: &ArrowType) -> DataType {
    match arrow {
        ArrowType::Boolean => DataType::Boolean,

        ArrowType::Int8 | ArrowType::Int16 | ArrowType::UInt8 => DataType::SmallInt,
        ArrowType::Int32 | ArrowType::UInt16 => DataType::Integer,
        ArrowType::Int64 | ArrowType::UInt32 | ArrowType::UInt64 => DataType::BigInt,

        ArrowType::Float16 | ArrowType::Float32 => DataType::Float,
        ArrowType::Float64 => DataType::Double,

        ArrowType::Decimal128(p, s) | ArrowType::Decimal256(p, s) => DataType::Decimal {
            precision: *p,
            scale: *s as u8,
        },

        ArrowType::Utf8 | ArrowType::LargeUtf8 | ArrowType::Utf8View => {
            DataType::Varchar { max_length: None }
        }

        ArrowType::Date32 | ArrowType::Date64 => DataType::Date,

        ArrowType::Time32(_) | ArrowType::Time64(_) => DataType::Time,

        ArrowType::Timestamp(_, None) => DataType::Timestamp {
            with_timezone: false,
        },
        ArrowType::Timestamp(_, Some(_)) => DataType::Timestamp {
            with_timezone: true,
        },

        ArrowType::Duration(_) | ArrowType::Interval(_) => DataType::Interval,

        ArrowType::Binary | ArrowType::LargeBinary | ArrowType::BinaryView => DataType::Blob,

        ArrowType::List(field) | ArrowType::LargeList(field) => {
            DataType::Array(Box::new(arrow_to_smelt(field.data_type())))
        }

        ArrowType::Struct(fields) => {
            let mapped_fields: Vec<(String, DataType)> = fields
                .iter()
                .map(|f| (f.name().clone(), arrow_to_smelt(f.data_type())))
                .collect();
            DataType::Struct(mapped_fields)
        }

        ArrowType::Null => DataType::Null,

        // Fallback for types we don't map (maps, unions, etc.)
        _ => DataType::unknown_dynamic(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{Field, TimeUnit};
    use std::sync::Arc;

    #[test]
    fn basic_mappings() {
        assert_eq!(arrow_to_smelt(&ArrowType::Boolean), DataType::Boolean);
        assert_eq!(arrow_to_smelt(&ArrowType::Int32), DataType::Integer);
        assert_eq!(arrow_to_smelt(&ArrowType::Int64), DataType::BigInt);
        assert_eq!(arrow_to_smelt(&ArrowType::Float64), DataType::Double);
        assert_eq!(
            arrow_to_smelt(&ArrowType::Utf8),
            DataType::Varchar { max_length: None }
        );
        assert_eq!(arrow_to_smelt(&ArrowType::Date32), DataType::Date);
        assert_eq!(
            arrow_to_smelt(&ArrowType::Timestamp(TimeUnit::Microsecond, None)),
            DataType::Timestamp {
                with_timezone: false
            }
        );
        assert_eq!(
            arrow_to_smelt(&ArrowType::Timestamp(
                TimeUnit::Microsecond,
                Some("UTC".into())
            )),
            DataType::Timestamp {
                with_timezone: true
            }
        );
        assert_eq!(
            arrow_to_smelt(&ArrowType::Decimal128(10, 2)),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
    }

    #[test]
    fn list_mapping() {
        let list_type = ArrowType::List(Arc::new(Field::new("item", ArrowType::Int32, true)));
        assert_eq!(
            arrow_to_smelt(&list_type),
            DataType::Array(Box::new(DataType::Integer))
        );
    }

    /// `LIST` → `Array<T>` for each base element type the property-test
    /// generators can now produce (ARRAY literals / ARRAY_AGG / subscript /
    /// slice) — guards the recursive `arrow_to_smelt` call in the List branch
    /// against silently regressing to a blanket `Unknown` for any of them.
    #[test]
    fn list_mapping_all_base_element_types() {
        let cases: &[(ArrowType, DataType)] = &[
            (ArrowType::Boolean, DataType::Boolean),
            (ArrowType::Int32, DataType::Integer),
            (ArrowType::Int64, DataType::BigInt),
            (ArrowType::Float64, DataType::Double),
            (ArrowType::Utf8, DataType::Varchar { max_length: None }),
            (ArrowType::Date32, DataType::Date),
            (
                ArrowType::Timestamp(TimeUnit::Microsecond, None),
                DataType::Timestamp {
                    with_timezone: false,
                },
            ),
            (
                ArrowType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                DataType::Timestamp {
                    with_timezone: true,
                },
            ),
            (
                ArrowType::Decimal128(10, 2),
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
            (ArrowType::Time64(TimeUnit::Microsecond), DataType::Time),
            (
                ArrowType::Interval(arrow::datatypes::IntervalUnit::MonthDayNano),
                DataType::Interval,
            ),
        ];

        for (elem_arrow, elem_smelt) in cases {
            let list_type = ArrowType::List(Arc::new(Field::new("item", elem_arrow.clone(), true)));
            assert_eq!(
                arrow_to_smelt(&list_type),
                DataType::Array(Box::new(elem_smelt.clone())),
                "LIST<{elem_arrow:?}> should map to Array<{elem_smelt:?}>"
            );
        }
    }

    /// `LargeList` uses the same recursive element mapping as `List`.
    #[test]
    fn large_list_mapping() {
        let list_type = ArrowType::LargeList(Arc::new(Field::new("item", ArrowType::Utf8, true)));
        assert_eq!(
            arrow_to_smelt(&list_type),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );
    }

    /// Nested `List<List<T>>` — element-type-aware recursion should reach two
    /// levels deep (e.g. `ARRAY_AGG(ARRAY_AGG(x))`-shaped results).
    #[test]
    fn nested_list_mapping() {
        let inner = ArrowType::List(Arc::new(Field::new("item", ArrowType::Int32, true)));
        let outer = ArrowType::List(Arc::new(Field::new("item", inner, true)));
        assert_eq!(
            arrow_to_smelt(&outer),
            DataType::Array(Box::new(DataType::Array(Box::new(DataType::Integer))))
        );
    }

    #[test]
    fn null_mapping() {
        assert_eq!(arrow_to_smelt(&ArrowType::Null), DataType::Null);
    }
}
