//! Type inferencer for smelt seed CSV files.
//!
//! Implements the type-precedence rules from `seeds.md §"Type inference"`:
//!
//! ```text
//! BOOLEAN → DATE → TIMESTAMP → INTEGER → DECIMAL → DOUBLE → VARCHAR
//! ```
//!
//! Two callers with the same code path:
//! - **Compile-time**: `sample_limit = Some(100)` (first 100 rows only)
//! - **Runtime**: `sample_limit = None` (all rows)
//!
//! Column with all-empty values → `DataType::Text` (VARCHAR).

use super::csv::CsvRecord;
use smelt_types::DataType;

/// Infer column types from a slice of `CsvRecord`s.
///
/// Returns a `Vec<(column_name, DataType)>` in header order.
///
/// `sample_limit` controls how many rows are examined:
/// - `Some(n)` → only the first `n` rows are used (compile-time inference).
/// - `None` → all rows are used (runtime inference).
pub fn infer_columns(
    rows: &[CsvRecord],
    headers: &[String],
    sample_limit: Option<usize>,
) -> Vec<(String, DataType)> {
    let sample: &[CsvRecord] = match sample_limit {
        Some(n) => &rows[..rows.len().min(n)],
        None => rows,
    };

    headers
        .iter()
        .enumerate()
        .map(|(col_idx, header)| {
            // Collect non-null values for this column.
            let values: Vec<&str> = sample
                .iter()
                .filter_map(|row| row.fields.get(col_idx).and_then(|v| v.as_deref()))
                .collect();

            let dtype = infer_type_from_values(&values);
            (header.clone(), dtype)
        })
        .collect()
}

/// Public re-export of the private type inference function, for use in
/// `mod.rs` when building `infer_csv_columns_with_pins`.
pub fn infer_type_from_values_pub(values: &[&str]) -> DataType {
    infer_type_from_values(values)
}

/// Infer a `DataType` from a sample of non-null CSV string values.
///
/// Precedence: BOOLEAN → DATE → TIMESTAMP → INTEGER → DECIMAL → DOUBLE → VARCHAR
///
/// If `values` is empty (column is all-NULL), returns `DataType::Text`.
fn infer_type_from_values(values: &[&str]) -> DataType {
    if values.is_empty() {
        return DataType::Text;
    }

    // BOOLEAN: all values are "true" or "false" (case-insensitive).
    if values
        .iter()
        .all(|v| matches!(v.to_lowercase().as_str(), "true" | "false"))
    {
        return DataType::Boolean;
    }

    // DATE: all values match `YYYY-MM-DD`.
    if values.iter().all(|v| looks_like_date(v.trim())) {
        return DataType::Date;
    }

    // TIMESTAMP: all values match `YYYY-MM-DD HH:MM:SS[.xxx]` (space separator only).
    // T-separated or TZ-suffix forms fall through to VARCHAR.
    if values.iter().all(|v| looks_like_timestamp(v.trim())) {
        return DataType::Timestamp {
            with_timezone: false,
        };
    }

    // INTEGER: all values parse as i64.
    if values.iter().all(|v| v.trim().parse::<i64>().is_ok()) {
        return DataType::Integer;
    }

    // DECIMAL: no scientific notation, digits + optional leading sign + optional one dot.
    // Compute max integer-digit count and max fractional-digit count across the sample.
    // Cap: p = int_digits + frac_digits ≤ 18, frac_digits ≤ 4.
    // Pure-integer strings are caught above by INTEGER; decimal literals have a dot.
    if let Some(dtype) = try_infer_decimal(values) {
        return dtype;
    }

    // DOUBLE: all values parse as f64 (including scientific notation).
    if values.iter().all(|v| v.trim().parse::<f64>().is_ok()) {
        return DataType::Double;
    }

    // VARCHAR fallback.
    DataType::Text
}

/// Try to infer `DECIMAL(p, s)` from a set of values.
///
/// Returns `Some(DataType::Decimal { precision, scale })` if all values are
/// valid decimal literals (digits, optional leading `-`, optional one `.`,
/// no scientific notation) and the cap `p ≤ 18, s ≤ 4` is satisfied.
///
/// Returns `Some(DataType::Double)` if all values are valid decimal literals
/// but the cap is exceeded (falls through to DOUBLE per spec).
///
/// Returns `None` if any value is not a decimal literal (lets the caller
/// fall through to DOUBLE or VARCHAR).
fn try_infer_decimal(values: &[&str]) -> Option<DataType> {
    let mut max_int_digits: u8 = 0;
    let mut max_frac_digits: u8 = 0;

    for v in values {
        let v = v.trim();
        // Strip leading sign.
        let v_unsigned = v.strip_prefix('-').unwrap_or(v);

        // Must contain only digits and at most one dot. Scientific notation
        // (contains 'e' or 'E') is excluded.
        if v_unsigned.is_empty() {
            return None;
        }
        if v_unsigned.contains('e') || v_unsigned.contains('E') {
            return None;
        }

        let dot_count = v_unsigned.chars().filter(|&c| c == '.').count();
        if dot_count > 1 {
            return None;
        }
        if !v_unsigned.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }

        if dot_count == 0 {
            // Integer-only decimal — integer digits only, no fractional part.
            // This would have been caught by INTEGER check already.
            // We still participate in DECIMAL tracking.
            let int_digits = v_unsigned.len() as u8;
            if int_digits > max_int_digits {
                max_int_digits = int_digits;
            }
        } else {
            // Has a fractional part.
            let (int_part, frac_part) = v_unsigned.split_once('.').unwrap();
            let int_digits = int_part.len() as u8;
            let frac_digits = frac_part.len() as u8;
            if int_digits > max_int_digits {
                max_int_digits = int_digits;
            }
            if frac_digits > max_frac_digits {
                max_frac_digits = frac_digits;
            }
        }
    }

    // At least one value must have a fractional part for DECIMAL to beat INTEGER.
    // If all are integers, INTEGER check above already handled them.
    if max_frac_digits == 0 {
        return None; // pure-integer set; already caught by INTEGER
    }

    let precision = max_int_digits + max_frac_digits;
    let scale = max_frac_digits;

    if precision > 18 || scale > 4 {
        // Exceeds cap → fall through to DOUBLE.
        Some(DataType::Double)
    } else {
        Some(DataType::Decimal { precision, scale })
    }
}

/// Returns `true` when `s` is shaped like `YYYY-MM-DD` with plausible field
/// ranges (year ∈ [1000,9999], month ∈ [1,12], day ∈ [1,31]).
/// Calendar correctness is not validated.
pub fn looks_like_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    parse_fixed_uint(parts[0], 4)
        .filter(|y| (1000..=9999).contains(y))
        .and(parse_fixed_uint(parts[1], 2).filter(|m| (1..=12).contains(m)))
        .and(parse_fixed_uint(parts[2], 2).filter(|d| (1..=31).contains(d)))
        .is_some()
}

/// Returns `true` when `s` is shaped like `YYYY-MM-DD HH:MM:SS[.xxx]` with a
/// **space** separator. T-separator or TZ-suffix forms return `false`.
pub fn looks_like_timestamp(s: &str) -> bool {
    // Must use space separator (not 'T').
    let (date_part, time_part) = match s.split_once(' ') {
        Some(parts) => parts,
        None => return false,
    };
    if !looks_like_date(date_part) {
        return false;
    }

    // Reject TZ suffixes: Z, +HH, -HH, +HH:MM, -HH:MM, etc.
    // We detect any trailing non-digit after stripping fractional seconds.
    let time_stripped = time_part
        .split_once('.')
        .map(|(h, _)| h)
        .unwrap_or(time_part);

    // The remaining string after stripping HH:MM:SS must be empty.
    let time_parts: Vec<&str> = time_stripped.split(':').collect();
    if time_parts.len() != 3 {
        return false;
    }

    // Each HH/MM/SS part must be exactly 2 digits.
    let h_ok = parse_fixed_uint(time_parts[0], 2)
        .filter(|h| *h <= 23)
        .is_some();
    let m_ok = parse_fixed_uint(time_parts[1], 2)
        .filter(|m| *m <= 59)
        .is_some();
    let s_ok = parse_fixed_uint(time_parts[2], 2)
        .filter(|sec| *sec <= 59)
        .is_some();

    if !(h_ok && m_ok && s_ok) {
        return false;
    }

    // Check: if there is a fractional part, the tail after the dot must be
    // pure digits (no TZ suffix like "Z", "+00", etc.).
    if let Some((_, frac)) = time_part.split_once('.') {
        if !frac.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if frac.is_empty() {
            return false;
        }
    }

    // Also reject if the time_part (before stripping frac) has trailing non-
    // digit characters that indicate a TZ suffix, like "08:00:00Z" or
    // "08:00:00+00".
    let last_char = time_stripped.chars().last().unwrap_or('0');
    if !last_char.is_ascii_digit() {
        return false;
    }

    true
}

/// Parse `s` as a non-negative integer, requiring `expected_len` ASCII digits.
fn parse_fixed_uint(s: &str, expected_len: usize) -> Option<u32> {
    if s.len() != expected_len || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_timestamp_space_separated() {
        assert!(looks_like_timestamp("2025-01-10 08:00:00"));
        assert!(looks_like_timestamp("2025-01-10 08:00:00.123"));
    }

    #[test]
    fn looks_like_timestamp_rejects_t_separator() {
        assert!(!looks_like_timestamp("2025-01-10T08:00:00"));
        assert!(!looks_like_timestamp("2025-01-10T08:00:00Z"));
    }

    #[test]
    fn looks_like_timestamp_rejects_tz_suffix() {
        assert!(!looks_like_timestamp("2025-01-10 08:00:00Z"));
        assert!(!looks_like_timestamp("2025-01-10 08:00:00+00"));
        assert!(!looks_like_timestamp("2025-01-10 08:00:00-05"));
    }

    #[test]
    fn decimal_cap_p_over_18() {
        // 19-digit integer: no dot, p=19 (but no frac, so INTEGER would catch it first).
        // Test with a fractional number where p=19.
        let values = ["123456789012345.6789"]; // int=15, frac=4, p=19 > 18
        assert_eq!(try_infer_decimal(&values), Some(DataType::Double));
    }

    #[test]
    fn decimal_cap_s_over_4() {
        let values = ["3.14159"]; // frac=5 > 4
        assert_eq!(try_infer_decimal(&values), Some(DataType::Double));
    }

    #[test]
    fn decimal_within_cap() {
        let values = ["123456.1234"]; // int=6, frac=4, p=10
        assert_eq!(
            try_infer_decimal(&values),
            Some(DataType::Decimal {
                precision: 10,
                scale: 4
            })
        );
    }
}
