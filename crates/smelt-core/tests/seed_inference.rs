//! Phase 4 TDD — type inferencer tests.
//!
//! These tests verify `smelt_core::seeds::infer::infer_columns` behaviour
//! against the type-precedence rules from `seeds.md §"Type inference"`:
//!
//!   BOOLEAN → DATE → TIMESTAMP → INTEGER → DECIMAL → DOUBLE → VARCHAR
//!
//! Compile-time inference: `sample_limit = Some(100)`.
//! Runtime inference: `sample_limit = None`.

use smelt_core::seeds::csv::{read_csv, CsvRecord};
use smelt_core::seeds::infer::infer_columns;
use smelt_types::DataType;
use std::io::Write;
use tempfile::NamedTempFile;

fn write_tmp(content: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content).unwrap();
    f.flush().unwrap();
    f
}

fn parse(content: &[u8]) -> (Vec<String>, Vec<CsvRecord>) {
    let f = write_tmp(content);
    let (headers, rows_iter) = read_csv(f.path()).unwrap();
    let rows = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();
    (headers, rows)
}

// ---------------------------------------------------------------------------
// detects_each_type
// ---------------------------------------------------------------------------

/// One column per type: BOOLEAN, INTEGER, DECIMAL(18,4), DOUBLE,
/// DATE, TIMESTAMP (space-separated), VARCHAR.
#[test]
fn detects_each_type() {
    let csv = b"\
bool_col,int_col,dec_col,double_col,date_col,ts_col,text_col\n\
true,42,3.1415,1.5e10,2025-01-10,2025-01-10 08:00:00,hello\n\
false,7,2.7183,2.7e8,2024-12-31,2024-12-31 23:59:59,world\n";

    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);

    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();

    assert_eq!(map["bool_col"], DataType::Boolean);
    assert_eq!(map["int_col"], DataType::Integer);
    // DECIMAL(p, s): p=5 (4 integer digits in "3.1415" → int_digits=1, frac=4; max: int=1, frac=4 → p=5,s=4)
    assert!(
        matches!(map["dec_col"], DataType::Decimal { scale, .. } if scale == 4),
        "dec_col should be Decimal with scale=4, got {:?}",
        map["dec_col"]
    );
    assert_eq!(map["double_col"], DataType::Double);
    assert_eq!(map["date_col"], DataType::Date);
    assert_eq!(
        map["ts_col"],
        DataType::Timestamp {
            with_timezone: false
        }
    );
    assert_eq!(map["text_col"], DataType::Text);
}

// ---------------------------------------------------------------------------
// precedence_order
// ---------------------------------------------------------------------------

/// Consolidated test covering three precedence sub-cases from `seeds.md`:
///
/// 1. Pure integers (`["1","2","3"]`) infer as INTEGER even though they are
///    technically valid DECIMAL(1,0) — INTEGER sits above DECIMAL in the chain.
/// 2. Scientific notation (`"1.5e10"`) → DOUBLE, not DECIMAL.
/// 3. ISO-8601 timestamp with TZ suffix (`"2025-01-10T08:00:00Z"`) → VARCHAR.
#[test]
fn precedence_order() {
    // Sub-case 1: pure integers must be INTEGER, not DECIMAL
    {
        let csv = b"x\n1\n2\n3\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert_eq!(
            map["x"],
            DataType::Integer,
            "pure integers should infer as Integer (not Decimal), got {:?}",
            map["x"]
        );
    }

    // Sub-case 2: scientific notation → DOUBLE
    {
        let csv = b"x\n1.5e10\n2.7e8\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert_eq!(
            map["x"],
            DataType::Double,
            "scientific notation should infer as Double, got {:?}",
            map["x"]
        );
    }

    // Sub-case 3: TZ-suffix timestamp → VARCHAR
    {
        let csv = b"x\n2025-01-10T08:00:00Z\n2025-01-11T09:00:00Z\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert_eq!(
            map["x"],
            DataType::Text,
            "TZ-suffix timestamp should fall through to Text, got {:?}",
            map["x"]
        );
    }
}

#[test]
fn int_plus_decimal_value_infers_decimal_not_integer() {
    // "3.14" doesn't parse as i64, so the column cannot be INTEGER.
    // DECIMAL is the next candidate; the column should infer as DECIMAL.
    let csv = b"x\n1\n2\n3.14\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert!(
        matches!(map["x"], DataType::Decimal { .. }),
        "mixed int+decimal should infer as Decimal, got {:?}",
        map["x"]
    );
}

#[test]
fn scientific_notation_infers_double_not_decimal() {
    // "1.5e10" contains 'e', which is not a valid DECIMAL literal per spec.
    // Should infer as DOUBLE.
    let csv = b"x\n1.5e10\n2.7e8\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert_eq!(
        map["x"],
        DataType::Double,
        "scientific notation should infer as Double, got {:?}",
        map["x"]
    );
}

#[test]
fn t_separator_with_tz_is_varchar() {
    // "2025-01-10T08:00:00Z" → T-separator + TZ suffix → VARCHAR.
    let csv = b"x\n2025-01-10T08:00:00Z\n2025-01-11T09:00:00Z\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert_eq!(
        map["x"],
        DataType::Text,
        "T-separated TZ timestamp should fall through to Text, got {:?}",
        map["x"]
    );
}

#[test]
fn t_separator_alone_is_varchar() {
    // "2025-01-10T08:00:00" → T-separator without TZ → VARCHAR.
    let csv = b"x\n2025-01-10T08:00:00\n2025-01-11T09:00:00\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert_eq!(
        map["x"],
        DataType::Text,
        "T-separated timestamp (no TZ) should fall through to Text, got {:?}",
        map["x"]
    );
}

// ---------------------------------------------------------------------------
// decimal_cap
// ---------------------------------------------------------------------------

/// Consolidated test for the three DECIMAL cap sub-cases from `seeds.md`:
///
/// 1. p=19 (19-digit integers, s=0) → DOUBLE (exceeds p≤18 cap)
/// 2. s=5 (5 fractional digits) → DOUBLE (exceeds s≤4 cap)
/// 3. DECIMAL(10, 4) stays DECIMAL (within both caps)
#[test]
fn decimal_cap() {
    // Sub-case 1: p=19, s=0 → DOUBLE
    {
        let csv = b"x\n1234567890123456789\n9876543210987654321\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert_eq!(
            map["x"],
            DataType::Double,
            "p=19 > 18 should fall to Double, got {:?}",
            map["x"]
        );
    }

    // Sub-case 2: s=5 → DOUBLE
    {
        let csv = b"x\n3.14159\n2.71828\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert_eq!(
            map["x"],
            DataType::Double,
            "s=5 > 4 should fall to Double, got {:?}",
            map["x"]
        );
    }

    // Sub-case 3: DECIMAL(10, 4) stays DECIMAL
    {
        let csv = b"x\n123456.1234\n99999.9999\n";
        let (headers, rows) = parse(csv);
        let inferred = infer_columns(&rows, &headers, None);
        let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
        assert!(
            matches!(map["x"], DataType::Decimal { precision: p, scale: 4 } if p <= 18),
            "DECIMAL(p≤18, 4) should stay Decimal, got {:?}",
            map["x"]
        );
    }
}

#[test]
fn decimal_p_over_18_falls_to_double() {
    // 19 integer digits + 0 frac → p=19 > 18 → DOUBLE.
    let csv = b"x\n1234567890123456789\n9876543210987654321\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    // 19-digit integers: can't fit in i64 (max ~9.2e18), but the spec says
    // INTEGER → i64. Since i64::MAX = 9223372036854775807 (19 digits but <
    // 9876543210987654321), these won't parse as i64. They won't be DECIMAL
    // (p=19 > 18), so they should fall to DOUBLE.
    assert_eq!(
        map["x"],
        DataType::Double,
        "p>18 decimal should fall to Double, got {:?}",
        map["x"]
    );
}

#[test]
fn decimal_s_over_4_falls_to_double() {
    // 5 fractional digits → s=5 > 4 → DOUBLE.
    let csv = b"x\n3.14159\n2.71828\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert_eq!(
        map["x"],
        DataType::Double,
        "s>4 decimal should fall to Double, got {:?}",
        map["x"]
    );
}

#[test]
fn decimal_within_cap_stays_decimal() {
    // DECIMAL(10, 4): p=10≤18, s=4≤4 → stays DECIMAL.
    let csv = b"x\n123456.1234\n99999.9999\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert!(
        matches!(map["x"], DataType::Decimal { precision: p, scale: 4 } if p <= 18),
        "DECIMAL(p≤18, 4) should stay Decimal, got {:?}",
        map["x"]
    );
}

// ---------------------------------------------------------------------------
// compile_runtime_widening
// ---------------------------------------------------------------------------

/// First 100 rows have integer values; row 101 has "3.14".
/// compile-time (limit=100) → INTEGER; runtime (limit=None) → DECIMAL.
#[test]
fn compile_runtime_widening() {
    // Build CSV: header + 100 integer rows + 1 decimal row
    let mut csv_bytes = b"x\n".to_vec();
    for i in 0..100u64 {
        csv_bytes.extend_from_slice(format!("{}\n", i).as_bytes());
    }
    csv_bytes.extend_from_slice(b"3.14\n");

    let f = write_tmp(&csv_bytes);
    let (headers, rows_iter) = read_csv(f.path()).unwrap();
    let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();

    // Compile-time: only sample first 100 rows → INTEGER
    let compile_inferred = infer_columns(&rows, &headers, Some(100));
    let compile_map: std::collections::HashMap<_, _> = compile_inferred.into_iter().collect();
    assert_eq!(
        compile_map["x"],
        DataType::Integer,
        "compile-time (first 100 rows all integers) should infer Integer, got {:?}",
        compile_map["x"]
    );

    // Runtime: all rows including row 101 → DECIMAL
    let runtime_inferred = infer_columns(&rows, &headers, None);
    let runtime_map: std::collections::HashMap<_, _> = runtime_inferred.into_iter().collect();
    assert!(
        matches!(runtime_map["x"], DataType::Decimal { .. }),
        "runtime (row 101 = 3.14) should infer Decimal, got {:?}",
        runtime_map["x"]
    );
}

// ---------------------------------------------------------------------------
// all_empty_column_is_varchar
// ---------------------------------------------------------------------------

#[test]
fn all_empty_column_is_varchar() {
    // A column where every value is empty (NULL) → VARCHAR.
    let csv = b"a,b,c\n1,,x\n2,,y\n3,,z\n";
    let (headers, rows) = parse(csv);
    let inferred = infer_columns(&rows, &headers, None);
    let map: std::collections::HashMap<_, _> = inferred.into_iter().collect();
    assert_eq!(
        map["b"],
        DataType::Text,
        "all-empty column should infer as Text (VARCHAR), got {:?}",
        map["b"]
    );
}
