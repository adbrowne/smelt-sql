//! Arrow `RecordBatch` builder for smelt seed CSV files.
//!
//! Takes parsed `CsvRecord`s and inferred column types, and produces
//! Arrow `RecordBatch`es with proper NULL handling.
//!
//! Type mapping (from `seeds.md §"Type inference"`):
//! - `DataType::Boolean` → `BooleanArray`
//! - `DataType::Integer` → `Int64Array`
//! - `DataType::Decimal { p, s }` → `Decimal128Array(p, s)`
//! - `DataType::Double` → `Float64Array`
//! - `DataType::Date` → `Date32Array` (days since Unix epoch)
//! - `DataType::Timestamp { with_timezone: false }` → `TimestampMicrosecondArray`
//! - `DataType::Text` → `StringArray`

use super::csv::CsvRecord;
use super::error::SeedError;
use super::infer::looks_like_date;
use arrow::array::{
    ArrayRef, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder,
    StringBuilder, TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, SchemaRef, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::{NaiveDate, NaiveDateTime};
use smelt_types::DataType;
use std::path::Path;
use std::sync::Arc;

/// Build Arrow `RecordBatch`es from parsed CSV rows and inferred column types.
///
/// `path` is used only for error messages.
/// `columns` is a list of `(column_name, DataType)` pairs — the inferred or
/// pinned schema. Must be in the same order as `CsvRecord::fields`.
pub fn to_arrow_batches(
    path: &Path,
    columns: &[(String, DataType)],
    rows: &[CsvRecord],
) -> Result<(SchemaRef, Vec<RecordBatch>), SeedError> {
    let fields: Vec<Field> = columns
        .iter()
        .map(|(name, dt)| {
            let arrow_dt = smelt_type_to_arrow(dt);
            Field::new(name, arrow_dt, true) // all seed columns are nullable
        })
        .collect();

    let schema = Arc::new(Schema::new(fields));

    if rows.is_empty() {
        let empty_batch = RecordBatch::new_empty(Arc::clone(&schema));
        return Ok((schema, vec![empty_batch]));
    }

    // Build one batch per column, then zip into a single RecordBatch.
    // We build column-by-column to keep memory usage predictable.
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());

    for (col_idx, (col_name, col_type)) in columns.iter().enumerate() {
        let array = build_column(path, col_idx, col_name, col_type, rows)?;
        arrays.push(array);
    }

    let batch = RecordBatch::try_new(Arc::clone(&schema), arrays).map_err(|e| SeedError::Io {
        path: path.to_path_buf(),
        message: format!("failed to build RecordBatch: {}", e),
    })?;

    Ok((schema, vec![batch]))
}

/// Map a smelt `DataType` to an Arrow `DataType`.
fn smelt_type_to_arrow(dt: &DataType) -> ArrowDataType {
    match dt {
        DataType::Boolean => ArrowDataType::Boolean,
        DataType::Integer => ArrowDataType::Int64,
        DataType::Decimal { precision, scale } => {
            ArrowDataType::Decimal128(*precision, *scale as i8)
        }
        DataType::Double => ArrowDataType::Float64,
        DataType::Date => ArrowDataType::Date32,
        DataType::Timestamp {
            with_timezone: false,
        } => ArrowDataType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Timestamp {
            with_timezone: true,
        } => ArrowDataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        // All other types (Text, Varchar, etc.) → Utf8
        _ => ArrowDataType::Utf8,
    }
}

/// Build a single Arrow array for one column.
fn build_column(
    path: &Path,
    col_idx: usize,
    col_name: &str,
    col_type: &DataType,
    rows: &[CsvRecord],
) -> Result<ArrayRef, SeedError> {
    match col_type {
        DataType::Boolean => {
            let mut builder = BooleanBuilder::with_capacity(rows.len());
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => match v.to_lowercase().as_str() {
                        "true" => builder.append_value(true),
                        "false" => builder.append_value(false),
                        _ => {
                            return Err(SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            })
                        }
                    },
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        DataType::Integer => {
            let mut builder = Int64Builder::with_capacity(rows.len());
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => match v.trim().parse::<i64>() {
                        Ok(n) => builder.append_value(n),
                        Err(_) => {
                            return Err(SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            })
                        }
                    },
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        DataType::Decimal { precision, scale } => {
            let scale_i8 = *scale as i8;
            let precision_u8 = *precision;
            let mut builder = Decimal128Builder::with_capacity(rows.len())
                .with_precision_and_scale(precision_u8, scale_i8)
                .map_err(|e| SeedError::Io {
                    path: path.to_path_buf(),
                    message: format!("invalid decimal precision/scale: {}", e),
                })?;
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => {
                        let decimal_val = parse_decimal(v, scale_i8).ok_or_else(|| {
                            SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            }
                        })?;
                        builder.append_value(decimal_val);
                    }
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        DataType::Double => {
            let mut builder = Float64Builder::with_capacity(rows.len());
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => match v.trim().parse::<f64>() {
                        Ok(n) => builder.append_value(n),
                        Err(_) => {
                            return Err(SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            })
                        }
                    },
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        DataType::Date => {
            let mut builder = Date32Builder::with_capacity(rows.len());
            let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => {
                        let date = NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d")
                            .ok()
                            .filter(|_d| looks_like_date(v.trim()))
                            .ok_or_else(|| SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            })?;
                        let days = (date - epoch).num_days() as i32;
                        builder.append_value(days);
                    }
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        DataType::Timestamp {
            with_timezone: false,
        } => {
            let mut builder = TimestampMicrosecondBuilder::with_capacity(rows.len());
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => {
                        // Try with optional fractional seconds.
                        let dt = parse_timestamp(v.trim()).ok_or_else(|| {
                            SeedError::TypeCoercionFailed {
                                path: path.to_path_buf(),
                                row: row.row_index,
                                column: col_name.to_string(),
                                value: v.to_string(),
                                expected: col_type.clone(),
                            }
                        })?;
                        builder.append_value(dt);
                    }
                }
            }
            Ok(Arc::new(builder.finish()))
        }

        // Text / Varchar / fallback → StringArray
        _ => {
            let mut builder = StringBuilder::with_capacity(rows.len(), rows.len() * 8);
            for row in rows {
                match row.fields.get(col_idx).and_then(|v| v.as_deref()) {
                    None => builder.append_null(),
                    Some(v) => builder.append_value(v),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
    }
}

/// Parse a decimal string value to an Arrow `Decimal128` (i128) scaled value.
///
/// e.g. "3.14" with scale=4 → 31400 (i.e. 3.14 * 10^4).
fn parse_decimal(v: &str, scale: i8) -> Option<i128> {
    let v = v.trim();
    // Reject scientific notation.
    if v.contains('e') || v.contains('E') {
        return None;
    }
    // Parse via rust_decimal for correctness.
    let negative = v.starts_with('-');
    let v_abs = if negative { &v[1..] } else { v };

    let (int_part, frac_part) = if let Some((i, f)) = v_abs.split_once('.') {
        (i, f)
    } else {
        (v_abs, "")
    };

    // Parse integer part.
    let int_val: i128 = if int_part.is_empty() {
        0
    } else {
        int_part.parse::<i128>().ok()?
    };

    // Parse fractional part, padded / truncated to `scale` digits.
    let scale_usize = scale as usize;
    let frac_val: i128 = if frac_part.is_empty() {
        0
    } else if frac_part.len() <= scale_usize {
        // Pad with trailing zeros.
        let padded = format!("{:0<width$}", frac_part, width = scale_usize);
        padded.parse::<i128>().ok()?
    } else {
        // Truncate extra digits.
        frac_part[..scale_usize].parse::<i128>().ok()?
    };

    let scale_factor: i128 = 10i128.pow(scale as u32);
    let result = int_val * scale_factor + frac_val;
    Some(if negative { -result } else { result })
}

/// Parse a timestamp string to microseconds since Unix epoch.
///
/// Accepts `YYYY-MM-DD HH:MM:SS` and `YYYY-MM-DD HH:MM:SS.fff`.
fn parse_timestamp(v: &str) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S%.f"))
        .ok()?;
    Some(dt.and_utc().timestamp_micros())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decimal_basic() {
        assert_eq!(parse_decimal("3.14", 4), Some(31400));
        assert_eq!(parse_decimal("3.1415", 4), Some(31415));
        assert_eq!(parse_decimal("-3.14", 4), Some(-31400));
        assert_eq!(parse_decimal("42", 4), Some(420000));
        assert_eq!(parse_decimal("1.5e10", 4), None); // scientific notation
    }

    #[test]
    fn smelt_type_to_arrow_mapping() {
        assert_eq!(
            smelt_type_to_arrow(&DataType::Integer),
            ArrowDataType::Int64
        );
        assert_eq!(
            smelt_type_to_arrow(&DataType::Double),
            ArrowDataType::Float64
        );
        assert_eq!(
            smelt_type_to_arrow(&DataType::Boolean),
            ArrowDataType::Boolean
        );
        assert_eq!(smelt_type_to_arrow(&DataType::Date), ArrowDataType::Date32);
        assert_eq!(
            smelt_type_to_arrow(&DataType::Timestamp {
                with_timezone: false
            }),
            ArrowDataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(smelt_type_to_arrow(&DataType::Text), ArrowDataType::Utf8);
    }
}
