//! Test runner: executes compiled test SQL and compares results.

use arrow::array::RecordBatch;
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
        expected_rows: Vec<BTreeMap<String, String>>,
        actual_rows: Vec<BTreeMap<String, String>>,
        missing: Vec<BTreeMap<String, String>>,
        unexpected: Vec<BTreeMap<String, String>>,
    },
    /// Row count mismatch
    RowCountMismatch { expected: usize, actual: usize },
    /// Compilation error
    CompilationError(String),
}

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestError::ExecutionError(msg) => write!(f, "SQL execution failed: {}", msg),
            TestError::CompilationError(msg) => write!(f, "Compilation failed: {}", msg),
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
pub fn compare_rows(
    actual: &[BTreeMap<String, String>],
    expected: &[BTreeMap<String, String>],
    check_order: bool,
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
            if !rows_match(actual_row, expected_row) {
                return Some(TestError::Mismatch {
                    expected_rows: expected.to_vec(),
                    actual_rows: filtered_actual.clone(),
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
                if !matched_expected[ei] && rows_match(actual_row, expected_row) {
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
                expected_rows: expected.to_vec(),
                actual_rows: filtered_actual,
                missing,
                unexpected,
            })
        }
    }
}

/// Check if two rows match (with numeric tolerance for floats).
fn rows_match(actual: &BTreeMap<String, String>, expected: &BTreeMap<String, String>) -> bool {
    for (key, expected_val) in expected {
        let actual_val = match actual.get(key) {
            Some(v) => v,
            None => return false,
        };
        if !values_match(actual_val, expected_val) {
            return false;
        }
    }
    true
}

/// Compare two string values with numeric tolerance.
fn values_match(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    // Try numeric comparison with epsilon
    if let (Ok(a), Ok(e)) = (actual.parse::<f64>(), expected.parse::<f64>()) {
        (a - e).abs() < 1e-6
    } else {
        false
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

    // Convert to comparable rows
    let actual_rows = batches_to_rows(&batches);
    let expected_rows = normalize_expected_rows(expected);

    // Compare
    let error = compare_rows(&actual_rows, &expected_rows, check_order);

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
        assert!(compare_rows(&[actual], &[expected], false).is_none());
    }

    #[test]
    fn test_compare_rows_mismatch() {
        let mut actual = BTreeMap::new();
        actual.insert("x".to_string(), "1".to_string());
        let mut expected = BTreeMap::new();
        expected.insert("x".to_string(), "2".to_string());
        assert!(compare_rows(&[actual], &[expected], false).is_some());
    }

    #[test]
    fn test_compare_rows_set_order_independent() {
        let mut r1 = BTreeMap::new();
        r1.insert("x".to_string(), "1".to_string());
        let mut r2 = BTreeMap::new();
        r2.insert("x".to_string(), "2".to_string());
        // Actual in reverse order of expected
        assert!(compare_rows(&[r2.clone(), r1.clone()], &[r1, r2], false).is_none());
    }

    #[test]
    fn test_compare_rows_ordered() {
        let mut r1 = BTreeMap::new();
        r1.insert("x".to_string(), "1".to_string());
        let mut r2 = BTreeMap::new();
        r2.insert("x".to_string(), "2".to_string());
        // Ordered comparison should fail when order differs
        assert!(compare_rows(&[r2.clone(), r1.clone()], &[r1, r2], true).is_some());
    }

    #[test]
    fn test_compare_rows_extra_columns_ignored() {
        let mut actual = BTreeMap::new();
        actual.insert("x".to_string(), "1".to_string());
        actual.insert("extra".to_string(), "ignored".to_string());
        let mut expected = BTreeMap::new();
        expected.insert("x".to_string(), "1".to_string());
        assert!(compare_rows(&[actual], &[expected], false).is_none());
    }

    #[test]
    fn test_values_match_numeric() {
        assert!(values_match("1.23", "1.23"));
        assert!(values_match("1.23000001", "1.23"));
        assert!(!values_match("3.5", "1.23"));
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
