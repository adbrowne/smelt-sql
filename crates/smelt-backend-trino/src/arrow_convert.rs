//! Trino type signature -> Arrow `DataType`, and JSON result rows -> Arrow
//! `RecordBatch`.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, Date32Builder, Decimal128Builder, Float32Builder, Float64Builder,
    Int32Builder, Int64Builder, RecordBatch, StringBuilder, TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use smelt_backend::BackendError;

use crate::protocol::Column;

/// Map a Trino type signature string (e.g. `"bigint"`, `"decimal(18,4)"`,
/// `"varchar(32)"`) to the Arrow `DataType` the client stores it as.
///
/// An unrecognised signature is a typed error naming the offending string,
/// never a silent fallback to `Utf8`/null (fail-loud discipline,
/// `CLAUDE.md` §"Fail-loud discipline").
pub fn trino_type_to_arrow(type_str: &str) -> Result<DataType, BackendError> {
    let type_str = type_str.trim();
    let (base, args) = match type_str.find('(') {
        Some(idx) if type_str.ends_with(')') => (
            &type_str[..idx],
            Some(&type_str[idx + 1..type_str.len() - 1]),
        ),
        _ => (type_str, None),
    };

    match base {
        "boolean" => Ok(DataType::Boolean),
        "integer" => Ok(DataType::Int32),
        "bigint" => Ok(DataType::Int64),
        "real" => Ok(DataType::Float32),
        "double" => Ok(DataType::Float64),
        "varchar" | "char" => Ok(DataType::Utf8),
        "date" => Ok(DataType::Date32),
        "timestamp" => Ok(DataType::Timestamp(TimeUnit::Microsecond, None)),
        "decimal" => {
            let args = args.ok_or_else(|| {
                BackendError::execution_failed(
                    "trino",
                    format!("decimal type signature missing precision/scale: {type_str}"),
                )
            })?;
            let mut parts = args.split(',');
            let precision: u8 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .ok_or_else(|| {
                    BackendError::execution_failed(
                        "trino",
                        format!("unparseable decimal precision in type signature: {type_str}"),
                    )
                })?;
            let scale: i8 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .ok_or_else(|| {
                    BackendError::execution_failed(
                        "trino",
                        format!("unparseable decimal scale in type signature: {type_str}"),
                    )
                })?;
            Ok(DataType::Decimal128(precision, scale))
        }
        _ => Err(BackendError::execution_failed(
            "trino",
            format!("unrecognised Trino type signature: {type_str}"),
        )),
    }
}

/// Parse a Trino-formatted decimal string (e.g. `"12345.6700"`, `"-3.5"`)
/// into the unscaled `i128` an Arrow `Decimal128` array stores.
fn parse_decimal_str(s: &str, scale: i8) -> Result<i128, BackendError> {
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    let scale = scale as usize;
    let mut frac = frac_part.to_string();
    if frac.len() > scale {
        frac.truncate(scale);
    } else {
        frac.push_str(&"0".repeat(scale - frac.len()));
    }
    let combined = format!("{int_part}{frac}");
    let magnitude: i128 = combined.parse().map_err(|e| {
        BackendError::execution_failed("trino", format!("unparseable decimal value '{s}': {e}"))
    })?;
    Ok(if negative { -magnitude } else { magnitude })
}

/// `NaiveDate::from_ymd_opt(1970, 1, 1)`'s day count in chrono's proleptic
/// Gregorian numbering (days since `0000-01-01`) — hoisted to a constant so
/// converting a date needs no fallible epoch construction at call time.
const UNIX_EPOCH_DAYS_FROM_CE: i32 = 719_163;

fn trino_date_to_days_since_epoch(s: &str) -> Result<i32, BackendError> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        BackendError::execution_failed("trino", format!("unparseable date value '{s}': {e}"))
    })?;
    Ok(date.num_days_from_ce() - UNIX_EPOCH_DAYS_FROM_CE)
}

fn trino_timestamp_to_micros(s: &str) -> Result<i64, BackendError> {
    let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").map_err(|e| {
        BackendError::execution_failed("trino", format!("unparseable timestamp value '{s}': {e}"))
    })?;
    Ok(dt.and_utc().timestamp_micros())
}

/// Decode one result page's `columns` metadata + `data` rows into a
/// `RecordBatch`. `data.len()` rows in, `data.len()` rows out — a JSON
/// `null` becomes an Arrow null in the same position, never a dropped row.
pub fn rows_to_record_batch(
    columns: &[Column],
    rows: &[Vec<serde_json::Value>],
) -> Result<RecordBatch, BackendError> {
    let data_types = columns
        .iter()
        .map(|c| trino_type_to_arrow(&c.raw_type))
        .collect::<Result<Vec<_>, _>>()?;

    let fields: Vec<Field> = columns
        .iter()
        .zip(&data_types)
        .map(|(c, dt)| Field::new(&c.name, dt.clone(), true))
        .collect();

    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());
    for (col_idx, data_type) in data_types.iter().enumerate() {
        arrays.push(build_column(data_type, rows, col_idx)?);
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, arrays).map_err(|e| {
        BackendError::execution_failed("trino", format!("failed to build record batch: {e}"))
    })
}

fn build_column(
    data_type: &DataType,
    rows: &[Vec<serde_json::Value>],
    col_idx: usize,
) -> Result<ArrayRef, BackendError> {
    macro_rules! build_scalar {
        ($builder_ty:ty, $extract:expr) => {{
            let mut builder = <$builder_ty>::new();
            for row in rows {
                match row.get(col_idx) {
                    None | Some(serde_json::Value::Null) => builder.append_null(),
                    Some(value) => {
                        let extracted: Option<_> = ($extract)(value);
                        match extracted {
                            Some(v) => builder.append_value(v),
                            None => {
                                return Err(BackendError::execution_failed(
                                    "trino",
                                    format!("unexpected JSON value for column {col_idx}: {value}"),
                                ))
                            }
                        }
                    }
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }};
    }

    match data_type {
        DataType::Boolean => build_scalar!(BooleanBuilder, |v: &serde_json::Value| v.as_bool()),
        DataType::Int32 => {
            build_scalar!(Int32Builder, |v: &serde_json::Value| v
                .as_i64()
                .map(|n| n as i32))
        }
        DataType::Int64 => build_scalar!(Int64Builder, |v: &serde_json::Value| v.as_i64()),
        DataType::Float32 => {
            build_scalar!(Float32Builder, |v: &serde_json::Value| v
                .as_f64()
                .map(|n| n as f32))
        }
        DataType::Float64 => build_scalar!(Float64Builder, |v: &serde_json::Value| v.as_f64()),
        DataType::Utf8 => {
            build_scalar!(StringBuilder, |v: &serde_json::Value| v
                .as_str()
                .map(|s| s.to_string()))
        }
        DataType::Date32 => {
            let mut builder = Date32Builder::with_capacity(rows.len());
            for row in rows {
                match row.get(col_idx) {
                    None | Some(serde_json::Value::Null) => builder.append_null(),
                    Some(serde_json::Value::String(s)) => {
                        builder.append_value(trino_date_to_days_since_epoch(s)?)
                    }
                    Some(other) => {
                        return Err(BackendError::execution_failed(
                            "trino",
                            format!("expected a date string, got: {other}"),
                        ))
                    }
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            let mut builder = TimestampMicrosecondBuilder::with_capacity(rows.len());
            for row in rows {
                match row.get(col_idx) {
                    None | Some(serde_json::Value::Null) => builder.append_null(),
                    Some(serde_json::Value::String(s)) => {
                        builder.append_value(trino_timestamp_to_micros(s)?)
                    }
                    Some(other) => {
                        return Err(BackendError::execution_failed(
                            "trino",
                            format!("expected a timestamp string, got: {other}"),
                        ))
                    }
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Decimal128(precision, scale) => {
            let mut builder = Decimal128Builder::with_capacity(rows.len())
                .with_precision_and_scale(*precision, *scale)
                .map_err(|e| {
                    BackendError::execution_failed(
                        "trino",
                        format!("invalid decimal precision/scale: {e}"),
                    )
                })?;
            for row in rows {
                match row.get(col_idx) {
                    None | Some(serde_json::Value::Null) => builder.append_null(),
                    Some(serde_json::Value::String(s)) => {
                        builder.append_value(parse_decimal_str(s, *scale)?)
                    }
                    Some(other) => {
                        return Err(BackendError::execution_failed(
                            "trino",
                            format!("expected a decimal string, got: {other}"),
                        ))
                    }
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        other => Err(BackendError::execution_failed(
            "trino",
            format!("no column builder for Arrow type: {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Column;
    use arrow::array::Array;
    use serde_json::json;

    fn col(name: &str, raw_type: &str) -> Column {
        Column {
            name: name.to_string(),
            raw_type: raw_type.to_string(),
            type_signature: None,
        }
    }

    #[test]
    fn maps_every_seed_type() {
        assert_eq!(trino_type_to_arrow("boolean").unwrap(), DataType::Boolean);
        assert_eq!(trino_type_to_arrow("integer").unwrap(), DataType::Int32);
        assert_eq!(trino_type_to_arrow("bigint").unwrap(), DataType::Int64);
        assert_eq!(trino_type_to_arrow("real").unwrap(), DataType::Float32);
        assert_eq!(trino_type_to_arrow("double").unwrap(), DataType::Float64);
        assert_eq!(
            trino_type_to_arrow("decimal(18,4)").unwrap(),
            DataType::Decimal128(18, 4)
        );
        assert_eq!(trino_type_to_arrow("varchar").unwrap(), DataType::Utf8);
        assert_eq!(trino_type_to_arrow("varchar(32)").unwrap(), DataType::Utf8);
        assert_eq!(trino_type_to_arrow("date").unwrap(), DataType::Date32);
        assert_eq!(
            trino_type_to_arrow("timestamp(6)").unwrap(),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    #[test]
    fn unknown_type_is_an_error_not_a_guess() {
        let err = trino_type_to_arrow("frobnicate(1,2)").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("frobnicate"));
    }

    #[test]
    fn json_values_become_typed_arrow_values() {
        let columns = vec![col("id", "bigint"), col("name", "varchar")];
        let rows = vec![
            vec![json!(1), json!("alice")],
            vec![json!(2), serde_json::Value::Null],
        ];
        let batch = rows_to_record_batch(&columns, &rows).unwrap();

        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert_eq!(ids.value(0), 1);
        assert_eq!(ids.value(1), 2);

        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "alice");
        assert!(names.is_null(1));
    }
}
