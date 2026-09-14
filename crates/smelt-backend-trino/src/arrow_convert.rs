//! Trino type signature -> Arrow `DataType`, and JSON result rows -> Arrow
//! `RecordBatch`. Also the write direction: Arrow `DataType` -> Trino DDL
//! type, and Arrow scalar value -> a typed SQL literal, for `load_table`.

use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, BooleanArray, BooleanBuilder, Date32Array, Date32Builder, Decimal128Array,
    Decimal128Builder, Float32Builder, Float64Array, Float64Builder, Int32Array, Int32Builder,
    Int64Array, Int64Builder, RecordBatch, StringArray, StringBuilder, TimestampMicrosecondArray,
    TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, IntervalUnit, Schema, TimeUnit};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime};
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

    // `INTERVAL DAY TO SECOND` / `INTERVAL YEAR TO MONTH` are spelled as a
    // multi-word, space-separated, uppercase signature with no parens — not
    // the `name` or `name(args)` shape every other base type here takes — so
    // they're matched by prefix before the `base` switch below, which would
    // otherwise see the whole multi-word string as one unrecognised `base`.
    if base.to_ascii_uppercase().starts_with("INTERVAL") {
        return Ok(DataType::Interval(IntervalUnit::MonthDayNano));
    }

    // `timestamp(3) with time zone` / `time(3) with time zone` — a trailing
    // `with time zone` suffix *after* the precision parens, which the
    // `find('(')`/`ends_with(')')` split above doesn't recognise (the string
    // doesn't end in `)`), so `base` above still holds the whole signature
    // including the suffix. Matched by substring before the `base` switch,
    // same reasoning as the `INTERVAL` prefix check above — found by the
    // live `dialect_audit` Trino schema leg probing `CURRENT_TIMESTAMP`/`NOW`.
    let lower = type_str.to_ascii_lowercase();
    if lower.contains("with time zone") {
        if lower.starts_with("timestamp") {
            return Ok(DataType::Timestamp(
                TimeUnit::Microsecond,
                Some("UTC".into()),
            ));
        }
        if lower.starts_with("time") {
            return Ok(DataType::Time64(TimeUnit::Microsecond));
        }
    }

    match base {
        "boolean" => Ok(DataType::Boolean),
        "integer" => Ok(DataType::Int32),
        "bigint" => Ok(DataType::Int64),
        "real" => Ok(DataType::Float32),
        "double" => Ok(DataType::Float64),
        // Trino's native JSON type carries no further Arrow representation
        // of its own; mapped to Utf8, matching how DuckDB's own JSON type
        // is read back through Arrow (see `duckdb_oracle.rs`'s
        // `json_type_check`) — both surface the serialized text.
        "varchar" | "char" | "json" => Ok(DataType::Utf8),
        "date" => Ok(DataType::Date32),
        "time" => Ok(DataType::Time64(TimeUnit::Microsecond)),
        "timestamp" => Ok(DataType::Timestamp(TimeUnit::Microsecond, None)),
        "array" => {
            let args = args.ok_or_else(|| {
                BackendError::execution_failed(
                    "trino",
                    format!("array type signature missing element type: {type_str}"),
                )
            })?;
            let elem_ty = trino_type_to_arrow(args)?;
            Ok(DataType::List(Arc::new(Field::new("item", elem_ty, true))))
        }
        "row" => {
            let args = args.ok_or_else(|| {
                BackendError::execution_failed(
                    "trino",
                    format!("row type signature missing fields: {type_str}"),
                )
            })?;
            let fields = split_top_level_commas(args)
                .into_iter()
                .map(|field_str| {
                    let (name, ty) = split_field_name_and_type(field_str.trim());
                    let arrow_ty = trino_type_to_arrow(ty)?;
                    Ok(Field::new(name.unwrap_or(""), arrow_ty, true))
                })
                .collect::<Result<Vec<_>, BackendError>>()?;
            Ok(DataType::Struct(fields.into()))
        }
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

/// Split `s` on top-level `,` — one not nested inside a `(...)` — so a
/// `row(...)`/`array(...)` type signature's own inner commas (from a nested
/// row or decimal argument list) don't get mistaken for a field separator.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Split one `row(...)` field entry into an optional field name and its type
/// signature. Trino spells a named row field as `name type` (space-separated
/// at top level, e.g. `"a integer"`, `"b row(c boolean)"`); an anonymous row
/// constructor's inferred signature has no name, just the bare type (e.g.
/// `"boolean"`). The split point is the first top-level space — one not
/// nested inside a `(...)`, so `"b row(c boolean)"` splits on the space
/// before `row`, not the one inside it.
fn split_field_name_and_type(s: &str) -> (Option<&str>, &str) {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ' ' if depth == 0 => return (Some(&s[..i]), s[i + 1..].trim()),
            _ => {}
        }
    }
    (None, s)
}

/// Map an Arrow `DataType` to the Trino DDL type `load_table` creates the
/// column as. Covers exactly the seed type set of `seeds.md` §"Type
/// inference" (matching `Backend::load_table`'s doc comment); anything else
/// is a typed error naming the offending type, never a silent `varchar`
/// fallback (fail-loud discipline, `CLAUDE.md` §"Fail-loud discipline").
///
/// `Timestamp(Microsecond, None)` maps to `timestamp(6)`, not bare
/// `timestamp` — Trino's bare `timestamp` defaults to second precision, which
/// would silently truncate the sub-second component on read-back.
pub fn arrow_type_to_trino_type(data_type: &DataType) -> Result<String, BackendError> {
    match data_type {
        DataType::Boolean => Ok("boolean".to_string()),
        DataType::Int32 => Ok("integer".to_string()),
        DataType::Int64 => Ok("bigint".to_string()),
        DataType::Float64 => Ok("double".to_string()),
        DataType::Date32 => Ok("date".to_string()),
        DataType::Timestamp(TimeUnit::Microsecond, None) => Ok("timestamp(6)".to_string()),
        DataType::Utf8 => Ok("varchar".to_string()),
        DataType::Decimal128(precision, scale) => Ok(format!("decimal({precision},{scale})")),
        other => Err(BackendError::unsupported(
            "trino",
            format!("load_table: no Trino DDL type for Arrow type {other:?}"),
        )),
    }
}

/// `NaiveDate::from_ymd_opt(1970, 1, 1)`'s day count in chrono's proleptic
/// Gregorian numbering (days since `0000-01-01`) — the write-direction
/// counterpart of [`trino_date_to_days_since_epoch`], reusing the same
/// constant as `trino_type_to_arrow`'s read direction.
fn days_since_epoch_to_date_string(days: i32) -> Result<String, BackendError> {
    let ce_days = days + UNIX_EPOCH_DAYS_FROM_CE;
    let date = NaiveDate::from_num_days_from_ce_opt(ce_days).ok_or_else(|| {
        BackendError::execution_failed("trino", format!("day count out of range: {days}"))
    })?;
    Ok(date.format("%Y-%m-%d").to_string())
}

/// Render the micros-since-epoch value of a `Timestamp(Microsecond, None)`
/// cell as a `YYYY-MM-DD HH:MM:SS.ffffff` string — always 6 fractional
/// digits, so the value round-trips through Trino's `timestamp(6)` exactly.
fn micros_to_timestamp_string(micros: i64) -> Result<String, BackendError> {
    let dt = DateTime::from_timestamp_micros(micros).ok_or_else(|| {
        BackendError::execution_failed(
            "trino",
            format!("microsecond timestamp out of range: {micros}"),
        )
    })?;
    Ok(dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
}

/// Render an unscaled `Decimal128` value as a plain numeric literal string
/// (e.g. `-3.5000` for `value = -35000, scale = 4`), unquoted — Trino parses
/// a decimal literal directly, with no surrounding type keyword needed
/// because the enclosing `CAST` supplies precision and scale.
fn decimal128_to_literal_string(value: i128, scale: i8) -> String {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let scale = scale as usize;
    let digits = magnitude.to_string();
    let padded = if digits.len() <= scale {
        format!("{}{digits}", "0".repeat(scale - digits.len() + 1))
    } else {
        digits
    };
    let split_at = padded.len() - scale;
    let (int_part, frac_part) = padded.split_at(split_at);
    let sign = if negative { "-" } else { "" };
    if scale == 0 {
        format!("{sign}{int_part}")
    } else {
        format!("{sign}{int_part}.{frac_part}")
    }
}

/// Escape a single-quoted SQL string literal value — doubling an embedded
/// `'` is standard SQL escaping, which Trino follows.
fn escape_string_value(s: &str) -> String {
    s.replace('\'', "''")
}

/// Render one cell of `array` at `row` as a SQL literal Trino types
/// unambiguously: `NULL` for a null cell (typed by the enclosing `CAST` in
/// the `INSERT` statement `load_table` builds), `DATE '…'` /
/// `TIMESTAMP '…'` for temporal values, a bare decimal/integer/boolean
/// literal, or a quoted, `''`-escaped string.
///
/// `data_type` must match `array`'s actual Arrow type — `load_table` always
/// calls this with the type it just downcast the array to, so a mismatch
/// here is an internal error, not a possible runtime input; it is still
/// reported as a typed `BackendError` rather than a panic (fail-loud
/// discipline, `CLAUDE.md` §"Fail-loud discipline").
pub fn render_trino_literal(
    array: &dyn Array,
    row: usize,
    data_type: &DataType,
) -> Result<String, BackendError> {
    /// Downcast or fail loud — every arm below calls this with a type that
    /// must already match `data_type`, so a failure here means the caller
    /// passed a mismatched `(array, data_type)` pair.
    fn downcast<'a, T: 'static>(
        array: &'a dyn Array,
        data_type: &DataType,
    ) -> Result<&'a T, BackendError> {
        array.as_any().downcast_ref::<T>().ok_or_else(|| {
            BackendError::execution_failed(
                "trino",
                format!("render_trino_literal: array does not match declared type {data_type:?}"),
            )
        })
    }

    if array.is_null(row) {
        return Ok("NULL".to_string());
    }
    match data_type {
        DataType::Boolean => {
            let a = downcast::<BooleanArray>(array, data_type)?;
            Ok(if a.value(row) { "TRUE" } else { "FALSE" }.to_string())
        }
        DataType::Int32 => {
            let a = downcast::<Int32Array>(array, data_type)?;
            Ok(a.value(row).to_string())
        }
        DataType::Int64 => {
            let a = downcast::<Int64Array>(array, data_type)?;
            Ok(a.value(row).to_string())
        }
        DataType::Float64 => {
            let a = downcast::<Float64Array>(array, data_type)?;
            Ok(a.value(row).to_string())
        }
        DataType::Date32 => {
            let a = downcast::<Date32Array>(array, data_type)?;
            let date_str = days_since_epoch_to_date_string(a.value(row))?;
            Ok(format!("DATE '{date_str}'"))
        }
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            let a = downcast::<TimestampMicrosecondArray>(array, data_type)?;
            let ts_str = micros_to_timestamp_string(a.value(row))?;
            Ok(format!("TIMESTAMP '{ts_str}'"))
        }
        DataType::Utf8 => {
            let a = downcast::<StringArray>(array, data_type)?;
            Ok(format!("'{}'", escape_string_value(a.value(row))))
        }
        DataType::Decimal128(_, scale) => {
            let a = downcast::<Decimal128Array>(array, data_type)?;
            Ok(decimal128_to_literal_string(a.value(row), *scale))
        }
        other => Err(BackendError::unsupported(
            "trino",
            format!("load_table: no literal renderer for Arrow type {other:?}"),
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

    /// `TIME` isn't in `load_table`'s write-direction seed type set (no
    /// `arrow_type_to_trino_type` arm), but the type-property oracle's
    /// schema leg (`TrinoOracle::query_types`) reads it back from a live
    /// `CAST(x AS TIME)` column — found by the `20260913-trino-emission`
    /// phase 9 live sweep, which surfaced it as an unmapped signature before
    /// this arm existed.
    /// `JSON` isn't in the write-direction seed type set either; found the
    /// same way `time`/`row`/`array` were, by the phase 9 live sweep hitting
    /// `JSON_EXTRACT(...)`.
    /// `INTERVAL DAY TO SECOND` / `INTERVAL YEAR TO MONTH` are Trino's
    /// multi-word, no-parens type signatures for `DATE`/`TIMESTAMP`
    /// arithmetic — found by the phase 9 live sweep on `ts - ts`.
    #[test]
    fn maps_interval_type() {
        assert_eq!(
            trino_type_to_arrow("INTERVAL DAY TO SECOND").unwrap(),
            DataType::Interval(IntervalUnit::MonthDayNano)
        );
        assert_eq!(
            trino_type_to_arrow("INTERVAL YEAR TO MONTH").unwrap(),
            DataType::Interval(IntervalUnit::MonthDayNano)
        );
    }

    /// `timestamp(3) with time zone` — Trino's zone-aware timestamp
    /// signature, returned by `CURRENT_TIMESTAMP`/`NOW`. Found by the live
    /// `dialect_audit` Trino schema leg.
    #[test]
    fn maps_timestamp_with_time_zone_type() {
        assert_eq!(
            trino_type_to_arrow("timestamp(3) with time zone").unwrap(),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
    }

    #[test]
    fn maps_json_type() {
        assert_eq!(trino_type_to_arrow("json").unwrap(), DataType::Utf8);
    }

    #[test]
    fn maps_time_type() {
        assert_eq!(
            trino_type_to_arrow("time(3)").unwrap(),
            DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            trino_type_to_arrow("time").unwrap(),
            DataType::Time64(TimeUnit::Microsecond)
        );
    }

    /// `ARRAY`/`ROW` aren't in `load_table`'s write-direction seed type set
    /// either, but the type-property oracle's schema leg reads them back
    /// from a live `ARRAY[...]`/`ROW(...)` column — found by the
    /// `20260913-trino-emission` phase 9 live sweep the same way `time` was.
    #[test]
    fn maps_array_type() {
        assert_eq!(
            trino_type_to_arrow("array(integer)").unwrap(),
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true)))
        );
    }

    #[test]
    fn maps_row_type_unnamed_fields() {
        // An anonymous ROW(...) constructor's inferred signature has no
        // field names — captured verbatim from the live sweep.
        assert_eq!(
            trino_type_to_arrow("row(boolean, integer)").unwrap(),
            DataType::Struct(
                vec![
                    Field::new("", DataType::Boolean, true),
                    Field::new("", DataType::Int32, true),
                ]
                .into()
            )
        );
    }

    #[test]
    fn maps_row_type_named_fields() {
        assert_eq!(
            trino_type_to_arrow("row(a integer, b varchar)").unwrap(),
            DataType::Struct(
                vec![
                    Field::new("a", DataType::Int32, true),
                    Field::new("b", DataType::Utf8, true),
                ]
                .into()
            )
        );
    }

    #[test]
    fn maps_nested_row_and_array_types() {
        assert_eq!(
            trino_type_to_arrow("row(a integer, b array(boolean))").unwrap(),
            DataType::Struct(
                vec![
                    Field::new("a", DataType::Int32, true),
                    Field::new(
                        "b",
                        DataType::List(Arc::new(Field::new("item", DataType::Boolean, true))),
                        true
                    ),
                ]
                .into()
            )
        );
        assert_eq!(
            trino_type_to_arrow("array(row(x bigint, y decimal(10,2)))").unwrap(),
            DataType::List(Arc::new(Field::new(
                "item",
                DataType::Struct(
                    vec![
                        Field::new("x", DataType::Int64, true),
                        Field::new("y", DataType::Decimal128(10, 2), true),
                    ]
                    .into()
                ),
                true
            )))
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

    #[test]
    fn arrow_type_to_trino_type_covers_the_seed_type_set() {
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Boolean).unwrap(),
            "boolean"
        );
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Int32).unwrap(),
            "integer"
        );
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Int64).unwrap(),
            "bigint"
        );
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Float64).unwrap(),
            "double"
        );
        assert_eq!(arrow_type_to_trino_type(&DataType::Date32).unwrap(), "date");
        // Not bare `timestamp` — Trino's default precision is seconds, which
        // would silently truncate the sub-second component on read-back.
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Timestamp(TimeUnit::Microsecond, None)).unwrap(),
            "timestamp(6)"
        );
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Utf8).unwrap(),
            "varchar"
        );
        assert_eq!(
            arrow_type_to_trino_type(&DataType::Decimal128(18, 4)).unwrap(),
            "decimal(18,4)"
        );
    }

    #[test]
    fn arrow_type_to_trino_type_refuses_an_unsupported_type() {
        let err = arrow_type_to_trino_type(&DataType::Float32).unwrap_err();
        assert!(matches!(err, BackendError::UnsupportedFeature { .. }));

        let list_type = DataType::List(Arc::new(Field::new("item", DataType::Int32, true)));
        let err = arrow_type_to_trino_type(&list_type).unwrap_err();
        assert!(matches!(err, BackendError::UnsupportedFeature { .. }));
    }

    #[test]
    fn renders_typed_literals_for_every_seed_type() {
        let bools: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), Some(false)]));
        assert_eq!(
            render_trino_literal(bools.as_ref(), 0, &DataType::Boolean).unwrap(),
            "TRUE"
        );
        assert_eq!(
            render_trino_literal(bools.as_ref(), 1, &DataType::Boolean).unwrap(),
            "FALSE"
        );

        let ints: ArrayRef = Arc::new(Int32Array::from(vec![42]));
        assert_eq!(
            render_trino_literal(ints.as_ref(), 0, &DataType::Int32).unwrap(),
            "42"
        );

        let bigints: ArrayRef = Arc::new(Int64Array::from(vec![9_000_000_000_i64]));
        assert_eq!(
            render_trino_literal(bigints.as_ref(), 0, &DataType::Int64).unwrap(),
            "9000000000"
        );

        let doubles: ArrayRef = Arc::new(Float64Array::from(vec![3.5]));
        assert_eq!(
            render_trino_literal(doubles.as_ref(), 0, &DataType::Float64).unwrap(),
            "3.5"
        );

        // 2024-01-15 is 19737 days since the Unix epoch.
        let dates: ArrayRef = Arc::new(Date32Array::from(vec![19737]));
        assert_eq!(
            render_trino_literal(dates.as_ref(), 0, &DataType::Date32).unwrap(),
            "DATE '2024-01-15'"
        );

        let timestamps: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
            1_705_318_861_123_456_i64,
        ]));
        assert_eq!(
            render_trino_literal(
                timestamps.as_ref(),
                0,
                &DataType::Timestamp(TimeUnit::Microsecond, None)
            )
            .unwrap(),
            "TIMESTAMP '2024-01-15 11:41:01.123456'"
        );

        let strings: ArrayRef = Arc::new(StringArray::from(vec!["hello"]));
        assert_eq!(
            render_trino_literal(strings.as_ref(), 0, &DataType::Utf8).unwrap(),
            "'hello'"
        );

        let decimals: ArrayRef = Arc::new(
            Decimal128Array::from(vec![-35000_i128])
                .with_precision_and_scale(18, 4)
                .unwrap(),
        );
        assert_eq!(
            render_trino_literal(decimals.as_ref(), 0, &DataType::Decimal128(18, 4)).unwrap(),
            "-3.5000"
        );

        let nullable_ints: ArrayRef = Arc::new(Int32Array::from(vec![None]));
        assert_eq!(
            render_trino_literal(nullable_ints.as_ref(), 0, &DataType::Int32).unwrap(),
            "NULL"
        );
    }

    #[test]
    fn escapes_a_string_literal_containing_a_quote() {
        let strings: ArrayRef = Arc::new(StringArray::from(vec!["it's a test"]));
        assert_eq!(
            render_trino_literal(strings.as_ref(), 0, &DataType::Utf8).unwrap(),
            "'it''s a test'"
        );
    }
}
