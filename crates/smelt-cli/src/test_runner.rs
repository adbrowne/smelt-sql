//! Test runner: executes compiled test SQL and compares results.

use arrow::array::RecordBatch;
use arrow::datatypes::DataType as ArrowDataType;
use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

/// Result of running a single test.
#[derive(Debug)]
pub struct TestResult {
    pub name: String,
    pub model: String,
    pub target_cte: Option<String>,
    pub passed: bool,
    pub duration: Duration,
    pub compiled_sql: String,
    pub error: Option<TestError>,
}

/// What went wrong in a test.
#[derive(Debug)]
pub enum TestError {
    /// SQL execution failed
    ExecutionError(String),
    /// Results didn't match
    Mismatch {
        missing: Vec<BTreeMap<String, String>>,
        unexpected: Vec<BTreeMap<String, String>>,
    },
    /// Row count mismatch
    RowCountMismatch { expected: usize, actual: usize },
    /// Compilation error
    CompilationError(String),
    /// Singular test returned rows (expected 0)
    SingularTestFailure {
        row_count: usize,
        sample_rows: Vec<BTreeMap<String, String>>,
    },
    /// Property-based test failure
    PropertyTestFailure {
        iteration: u32,
        total: u32,
        seed: u64,
        inner: Box<TestError>,
        generated_values: BTreeMap<String, Vec<BTreeMap<String, String>>>,
    },
    /// Anchored diagnostic from whole-query test inlining.
    ///
    /// Carries a diagnostic code (e.g. `"AmbiguousTestModel"`) and a
    /// human-readable message that may include a `file:line:col` prefix for
    /// terminal anchoring.
    InliningDiagnostic { code: &'static str, message: String },
}

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestError::ExecutionError(msg) => write!(f, "SQL execution failed: {}", msg),
            TestError::CompilationError(msg) => write!(f, "Compilation failed: {}", msg),
            TestError::InliningDiagnostic { code: _, message } => write!(f, "{}", message),
            TestError::RowCountMismatch { expected, actual } => {
                write!(f, "Expected {} row(s), got {} row(s)", expected, actual)
            }
            TestError::Mismatch {
                missing,
                unexpected,
                ..
            } => {
                if !missing.is_empty() {
                    writeln!(f, "  Missing rows (expected but not found):")?;
                    for row in missing {
                        writeln!(f, "    {:?}", row)?;
                    }
                }
                if !unexpected.is_empty() {
                    writeln!(f, "  Unexpected rows (found but not expected):")?;
                    for row in unexpected {
                        writeln!(f, "    {:?}", row)?;
                    }
                }
                Ok(())
            }
            TestError::SingularTestFailure {
                row_count,
                sample_rows,
            } => {
                writeln!(
                    f,
                    "  Singular test returned {} row(s) (expected 0).",
                    row_count
                )?;
                if !sample_rows.is_empty() {
                    writeln!(f, "  First failing rows:")?;
                    for row in sample_rows.iter().take(5) {
                        writeln!(f, "    {:?}", row)?;
                    }
                }
                Ok(())
            }
            TestError::PropertyTestFailure {
                iteration,
                total,
                seed,
                inner,
                generated_values,
            } => {
                writeln!(
                    f,
                    "  Property test failed on iteration {}/{}",
                    iteration, total
                )?;
                writeln!(
                    f,
                    "  Failing seed: 0x{:X} (reproduce with: smelt test --seed 0x{:X})",
                    seed, seed
                )?;
                if !generated_values.is_empty() {
                    writeln!(f, "  Generated values:")?;
                    for (dep, rows) in generated_values {
                        writeln!(f, "    {}:", dep)?;
                        for row in rows {
                            writeln!(f, "      {:?}", row)?;
                        }
                    }
                }
                write!(f, "{}", inner)?;
                Ok(())
            }
        }
    }
}

/// Execute a compiled test SQL string against an in-memory DuckDB and return RecordBatches.
#[cfg(feature = "duckdb")]
pub fn execute_test_sql(sql: &str) -> Result<Vec<RecordBatch>, String> {
    let conn = duckdb::Connection::open_in_memory()
        .map_err(|e| format!("Failed to open DuckDB: {}", e))?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("Failed to prepare SQL: {}", e))?;
    let batches: Vec<RecordBatch> = stmt
        .query_arrow([])
        .map_err(|e| format!("Failed to execute SQL: {}", e))?
        .collect();
    Ok(batches)
}

/// Execute SQL against a file-based DuckDB database and return RecordBatches.
#[cfg(feature = "duckdb")]
pub fn execute_sql_against_db(
    sql: &str,
    database_path: &std::path::Path,
) -> Result<Vec<RecordBatch>, String> {
    let conn = duckdb::Connection::open(database_path)
        .map_err(|e| format!("Failed to open DuckDB at {:?}: {}", database_path, e))?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("Failed to prepare SQL: {}", e))?;
    let batches: Vec<RecordBatch> = stmt
        .query_arrow([])
        .map_err(|e| format!("Failed to execute SQL: {}", e))?
        .collect();
    Ok(batches)
}

/// Run a singular test: execute SQL against a real database, pass if 0 rows returned.
#[cfg(feature = "duckdb")]
pub fn run_singular_test(
    test_name: &str,
    compiled_sql: &str,
    database_path: Option<&std::path::Path>,
) -> TestResult {
    let start = Instant::now();

    let batches = match database_path {
        Some(path) => execute_sql_against_db(compiled_sql, path),
        None => execute_test_sql(compiled_sql),
    };

    let batches = match batches {
        Ok(b) => b,
        Err(e) => {
            return TestResult {
                name: test_name.to_string(),
                model: String::new(),
                target_cte: None,
                passed: false,
                duration: start.elapsed(),
                compiled_sql: compiled_sql.to_string(),
                error: Some(TestError::ExecutionError(e)),
            };
        }
    };

    let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();
    let passed = row_count == 0;

    let error = if passed {
        None
    } else {
        let sample_rows = batches_to_rows(&batches);
        Some(TestError::SingularTestFailure {
            row_count,
            sample_rows: sample_rows.into_iter().take(10).collect(),
        })
    };

    TestResult {
        name: test_name.to_string(),
        model: String::new(),
        target_cte: None,
        passed,
        duration: start.elapsed(),
        compiled_sql: compiled_sql.to_string(),
        error,
    }
}

/// Convert Arrow RecordBatches to a list of row maps (column_name -> string value).
pub fn batches_to_rows(batches: &[RecordBatch]) -> Vec<BTreeMap<String, String>> {
    let mut rows = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        for row_idx in 0..batch.num_rows() {
            let mut row = BTreeMap::new();
            for (col_idx, field) in schema.fields().iter().enumerate() {
                let col = batch.column(col_idx);
                let value = arrow::util::display::array_value_to_string(col, row_idx)
                    .unwrap_or_else(|_| "ERROR".to_string());
                row.insert(field.name().clone(), value);
            }
            rows.push(row);
        }
    }
    rows
}

/// Extract Arrow column types from RecordBatches, keyed by column name.
///
/// Used by `compare_rows` to apply type-aware value matching (D-44): DECIMAL
/// columns are compared exactly; FLOAT/DOUBLE columns use the 1e-6 tolerance.
pub fn batches_to_column_types(batches: &[RecordBatch]) -> BTreeMap<String, ArrowDataType> {
    let mut types = BTreeMap::new();
    if let Some(batch) = batches.first() {
        for field in batch.schema().fields() {
            types.insert(field.name().clone(), field.data_type().clone());
        }
    }
    types
}

/// Convert YAML expected rows to string maps for comparison.
///
/// Values are normalized to match Arrow's string representation:
/// - integers: "1", "100"
/// - floats: "100.0" (ensure decimal point)
/// - strings: as-is
/// - booleans: "true"/"false"
/// - null: "" (empty string, matching Arrow's null display)
pub fn normalize_expected_rows(
    expected: &[BTreeMap<String, serde_yaml::Value>],
) -> Vec<BTreeMap<String, String>> {
    expected
        .iter()
        .map(|row| {
            row.iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_yaml::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                i.to_string()
                            } else if let Some(f) = n.as_f64() {
                                format_float(f)
                            } else {
                                n.to_string()
                            }
                        }
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Null => String::new(),
                        _ => format!("{:?}", v),
                    };
                    (k.clone(), s)
                })
                .collect()
        })
        .collect()
}

/// Format a float to ensure it has a decimal point (matching DuckDB output).
fn format_float(f: f64) -> String {
    let s = f.to_string();
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// Compare actual rows against expected rows.
///
/// If `check_order` is true, compares row by row positionally.
/// If false (default), treats both sides as sets (sorts before comparing).
///
/// Only compares columns that appear in `expected` -- extra columns in actual are ignored.
///
/// `column_types` maps column names to Arrow DataTypes so DECIMAL columns can be
/// compared exactly (D-44), while FLOAT/DOUBLE columns still use the 1e-6 tolerance.
pub fn compare_rows(
    actual: &[BTreeMap<String, String>],
    expected: &[BTreeMap<String, String>],
    check_order: bool,
    column_types: &BTreeMap<String, ArrowDataType>,
) -> Option<TestError> {
    if expected.is_empty() && actual.is_empty() {
        return None;
    }

    if actual.len() != expected.len() {
        return Some(TestError::RowCountMismatch {
            expected: expected.len(),
            actual: actual.len(),
        });
    }

    // Filter actual rows to only include columns from expected
    let expected_columns: Vec<String> = if let Some(first) = expected.first() {
        first.keys().cloned().collect()
    } else {
        return None;
    };

    let filtered_actual: Vec<BTreeMap<String, String>> = actual
        .iter()
        .map(|row| {
            expected_columns
                .iter()
                .map(|col| {
                    let val = row.get(col).cloned().unwrap_or_default();
                    (col.clone(), val)
                })
                .collect()
        })
        .collect();

    if check_order {
        // Ordered comparison
        for (i, (actual_row, expected_row)) in
            filtered_actual.iter().zip(expected.iter()).enumerate()
        {
            if !rows_match(actual_row, expected_row, column_types) {
                return Some(TestError::Mismatch {
                    missing: vec![expected_row.clone()],
                    unexpected: vec![filtered_actual[i].clone()],
                });
            }
        }
        None
    } else {
        // Set comparison: O(n²) with numeric-aware matching
        let mut matched_expected = vec![false; expected.len()];
        let mut unexpected = Vec::new();

        for actual_row in &filtered_actual {
            let mut found = false;
            for (ei, expected_row) in expected.iter().enumerate() {
                if !matched_expected[ei] && rows_match(actual_row, expected_row, column_types) {
                    matched_expected[ei] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                unexpected.push(actual_row.clone());
            }
        }

        let missing: Vec<_> = expected
            .iter()
            .enumerate()
            .filter(|(i, _)| !matched_expected[*i])
            .map(|(_, row)| row.clone())
            .collect();

        if missing.is_empty() && unexpected.is_empty() {
            None
        } else {
            Some(TestError::Mismatch {
                missing,
                unexpected,
            })
        }
    }
}

/// Check if two rows match, using type-aware comparison for each column.
fn rows_match(
    actual: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
    column_types: &BTreeMap<String, ArrowDataType>,
) -> bool {
    for (key, expected_val) in expected {
        let actual_val = match actual.get(key) {
            Some(v) => v,
            None => return false,
        };
        if !values_match(actual_val, expected_val, column_types.get(key)) {
            return false;
        }
    }
    true
}

/// Compare two string values.
///
/// For DECIMAL columns with non-zero scale (`Decimal128(_, s)` where `s > 0`):
/// exact string equality only — no float tolerance (D-44). Scale-0 decimals
/// (e.g., DuckDB HUGEINT = Decimal128(38, 0)) are integers and still use tolerance
/// so that "300" and "300.0" compare equal. For FLOAT/DOUBLE and unknown types:
/// 1e-6 relative epsilon tolerance.
fn values_match(actual: &str, expected: &str, col_type: Option<&ArrowDataType>) -> bool {
    if actual == expected {
        return true;
    }
    // Scaled DECIMAL columns (scale > 0): exact comparison only. D-44.
    if is_scaled_decimal(col_type) {
        return false;
    }
    // FLOAT/DOUBLE, integer-equivalent, and unknown: relative epsilon tolerance.
    if let (Ok(a), Ok(e)) = (actual.parse::<f64>(), expected.parse::<f64>()) {
        let diff = (a - e).abs();
        let scale = e.abs().max(a.abs()).max(1.0);
        diff / scale < 1e-6
    } else {
        false
    }
}

/// Returns true when the Arrow DataType is a Decimal with non-zero scale.
///
/// DuckDB HUGEINT → Decimal128(38, 0) has scale 0 and behaves as an integer —
/// it is NOT a scaled decimal, so it does not get exact-only comparison.
fn is_scaled_decimal(col_type: Option<&ArrowDataType>) -> bool {
    match col_type {
        Some(ArrowDataType::Decimal128(_, s)) => *s > 0,
        Some(ArrowDataType::Decimal256(_, s)) => *s > 0,
        _ => false,
    }
}

/// Run a single test: compile, execute, compare.
#[cfg(feature = "duckdb")]
pub fn run_test(
    test_name: &str,
    model_name: &str,
    target_cte: Option<&str>,
    compiled_sql: &str,
    expected: &[BTreeMap<String, serde_yaml::Value>],
    check_order: bool,
) -> TestResult {
    let start = Instant::now();

    // Execute
    let batches = match execute_test_sql(compiled_sql) {
        Ok(b) => b,
        Err(e) => {
            return TestResult {
                name: test_name.to_string(),
                model: model_name.to_string(),
                target_cte: target_cte.map(|s| s.to_string()),
                passed: false,
                duration: start.elapsed(),
                compiled_sql: compiled_sql.to_string(),
                error: Some(TestError::ExecutionError(e)),
            };
        }
    };

    // Extract column types for type-aware comparison (D-44: DECIMAL exact, float tolerant).
    let column_types = batches_to_column_types(&batches);

    // Convert to comparable rows
    let actual_rows = batches_to_rows(&batches);
    let expected_rows = normalize_expected_rows(expected);

    // Compare
    let error = compare_rows(&actual_rows, &expected_rows, check_order, &column_types);

    TestResult {
        name: test_name.to_string(),
        model: model_name.to_string(),
        target_cte: target_cte.map(|s| s.to_string()),
        passed: error.is_none(),
        duration: start.elapsed(),
        compiled_sql: compiled_sql.to_string(),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_expected_rows() {
        let mut row = BTreeMap::new();
        row.insert(
            "count".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(42)),
        );
        row.insert(
            "avg".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1.23)),
        );
        row.insert(
            "name".to_string(),
            serde_yaml::Value::String("Alice".to_string()),
        );
        let rows = normalize_expected_rows(&[row]);
        assert_eq!(rows[0]["count"], "42");
        assert_eq!(rows[0]["avg"], "1.23");
        assert_eq!(rows[0]["name"], "Alice");
    }

    #[test]
    fn test_compare_rows_matching() {
        let mut actual = BTreeMap::new();
        actual.insert("x".to_string(), "1".to_string());
        actual.insert("y".to_string(), "hello".to_string());
        let mut expected = BTreeMap::new();
        expected.insert("x".to_string(), "1".to_string());
        expected.insert("y".to_string(), "hello".to_string());
        assert!(compare_rows(&[actual], &[expected], false, &BTreeMap::new()).is_none());
    }

    #[test]
    fn test_compare_rows_mismatch() {
        let mut actual = BTreeMap::new();
        actual.insert("x".to_string(), "1".to_string());
        let mut expected = BTreeMap::new();
        expected.insert("x".to_string(), "2".to_string());
        assert!(compare_rows(&[actual], &[expected], false, &BTreeMap::new()).is_some());
    }

    #[test]
    fn test_compare_rows_set_order_independent() {
        let mut r1 = BTreeMap::new();
        r1.insert("x".to_string(), "1".to_string());
        let mut r2 = BTreeMap::new();
        r2.insert("x".to_string(), "2".to_string());
        // Actual in reverse order of expected
        assert!(compare_rows(
            &[r2.clone(), r1.clone()],
            &[r1, r2],
            false,
            &BTreeMap::new()
        )
        .is_none());
    }

    #[test]
    fn test_compare_rows_ordered() {
        let mut r1 = BTreeMap::new();
        r1.insert("x".to_string(), "1".to_string());
        let mut r2 = BTreeMap::new();
        r2.insert("x".to_string(), "2".to_string());
        // Ordered comparison should fail when order differs
        assert!(
            compare_rows(&[r2.clone(), r1.clone()], &[r1, r2], true, &BTreeMap::new()).is_some()
        );
    }

    #[test]
    fn test_compare_rows_extra_columns_ignored() {
        let mut actual = BTreeMap::new();
        actual.insert("x".to_string(), "1".to_string());
        actual.insert("extra".to_string(), "ignored".to_string());
        let mut expected = BTreeMap::new();
        expected.insert("x".to_string(), "1".to_string());
        assert!(compare_rows(&[actual], &[expected], false, &BTreeMap::new()).is_none());
    }

    #[test]
    fn test_values_match_numeric() {
        assert!(values_match("1.23", "1.23", None));
        assert!(values_match("1.23000001", "1.23", None));
        assert!(!values_match("3.5", "1.23", None));
    }

    #[test]
    fn test_values_match_decimal_exact() {
        // D-44: scaled DECIMAL columns use exact comparison (no tolerance).
        let decimal_type = ArrowDataType::Decimal128(10, 7);
        // Exact match still passes.
        assert!(values_match("1.0000001", "1.0000001", Some(&decimal_type)));
        // Tiny diff that float tolerance would accept must FAIL for scaled DECIMAL.
        assert!(!values_match("1.0000001", "1.0000000", Some(&decimal_type)));
        // Decimal256 same behaviour.
        let dec256 = ArrowDataType::Decimal256(30, 10);
        assert!(!values_match(
            "1.00000000001",
            "1.00000000000",
            Some(&dec256)
        ));
        // Scale-0 decimal (HUGEINT = Decimal128(38,0)) still uses float tolerance.
        let hugeint = ArrowDataType::Decimal128(38, 0);
        assert!(values_match("300", "300.0", Some(&hugeint)));
        assert!(!values_match("301", "300.0", Some(&hugeint)));
    }

    #[test]
    fn test_values_match_float_tolerance_preserved() {
        // Float/Double columns retain the 1e-6 relative tolerance.
        let float64 = ArrowDataType::Float64;
        assert!(values_match("1.0000001", "1.0", Some(&float64)));
        let float32 = ArrowDataType::Float32;
        assert!(values_match("1.0000001", "1.0", Some(&float32)));
        // Large diff still fails.
        assert!(!values_match("2.0", "1.0", Some(&float64)));
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn test_execute_test_sql() {
        let sql = "SELECT 1 as x, 'hello' as y";
        let batches = execute_test_sql(sql).unwrap();
        let rows = batches_to_rows(&batches);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["x"], "1");
        assert_eq!(rows[0]["y"], "hello");
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn test_run_test_pass() {
        let mut expected = BTreeMap::new();
        expected.insert(
            "x".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        let result = run_test("test1", "model1", None, "SELECT 1 as x", &[expected], false);
        assert!(result.passed);
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn test_run_test_fail() {
        let mut expected = BTreeMap::new();
        expected.insert(
            "x".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(999)),
        );
        let result = run_test("test1", "model1", None, "SELECT 1 as x", &[expected], false);
        assert!(!result.passed);
    }
}
