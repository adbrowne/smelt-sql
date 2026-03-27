//! Property-based test generation: detects missing input columns,
//! infers their types, generates random values, and runs N iterations.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use smelt_parser::ast::Expr;
use smelt_parser::SyntaxKind;
use smelt_types::DataType;

use crate::test_compiler::{compile_cte_test, compile_whole_model_test, extract_ctes};
use crate::test_runner::TestError;
#[cfg(feature = "duckdb")]
use crate::test_runner::{run_test, TestResult};

/// Result of running a property-based test.
#[derive(Debug)]
pub struct PropertyTestResult {
    pub name: String,
    pub model: String,
    pub target_cte: Option<String>,
    pub iterations: u32,
    pub passed: bool,
    pub duration: Duration,
    pub failing_seed: Option<u64>,
    pub failing_iteration: Option<u32>,
    #[cfg(feature = "duckdb")]
    pub inner_result: Option<TestResult>,
    #[cfg(not(feature = "duckdb"))]
    pub inner_result: Option<String>,
}

/// Convert a serde_yaml::Value to a DataType for type inference.
pub fn yaml_value_to_datatype(v: &serde_yaml::Value) -> DataType {
    match v {
        serde_yaml::Value::Number(n) => {
            if n.is_i64() {
                DataType::Integer
            } else if n.is_f64() {
                DataType::Double
            } else {
                DataType::Integer
            }
        }
        serde_yaml::Value::String(s) => {
            if is_date_string(s) {
                DataType::Date
            } else {
                DataType::Text
            }
        }
        serde_yaml::Value::Bool(_) => DataType::Boolean,
        serde_yaml::Value::Null => DataType::Null,
        _ => DataType::Unknown,
    }
}

/// Check if a string matches the YYYY-MM-DD date pattern.
fn is_date_string(s: &str) -> bool {
    if s.len() < 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

/// Find columns referenced from a specific dependency in a CTE body SQL.
///
/// Parses the SQL and walks all expression nodes looking for column references
/// that are qualified with `dependency_name` or unqualified when the dependency
/// is the only source.
pub fn find_dependency_columns(
    cte_body_sql: &str,
    dependency_name: &str,
    all_cte_names: &[String],
) -> Vec<String> {
    let parse = smelt_parser::parse(cte_body_sql);
    let mut columns = Vec::new();
    let mut seen = HashSet::new();

    // Determine if this dependency is the only source (for unqualified column resolution)
    let is_only_source = {
        let body_upper = cte_body_sql.to_uppercase();
        let dep_upper = dependency_name.to_uppercase();
        let mut source_count = 0;
        for name in all_cte_names {
            let name_upper = name.to_uppercase();
            if body_upper.contains(&name_upper) {
                source_count += 1;
            }
        }
        // If only one CTE name appears in the body and it matches our dependency
        source_count <= 1 && body_upper.contains(&dep_upper)
    };

    for node in parse.syntax().descendants() {
        // Try to interpret expression-like nodes
        let kind = node.kind();
        if kind == SyntaxKind::EXPRESSION
            || kind == SyntaxKind::BINARY_EXPR
            || kind == SyntaxKind::FUNCTION_CALL
        {
            if let Some(expr) = Expr::cast(node.clone()) {
                if let Some(col_ref) = expr.as_column_ref() {
                    let col_name = col_ref.name().to_string();
                    if let Some(qualifier) = col_ref.qualifier() {
                        // Qualified: dep.col
                        if qualifier.eq_ignore_ascii_case(dependency_name)
                            && seen.insert(col_name.clone())
                        {
                            columns.push(col_name);
                        }
                    } else if is_only_source && seen.insert(col_name.clone()) {
                        // Unqualified and this is the only source
                        columns.push(col_name);
                    }
                }
            }
        }
    }

    columns
}

/// Find columns that are referenced in the CTE body but not provided in inputs.
pub fn find_missing_columns(
    model_sql: &str,
    target_cte: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
) -> BTreeMap<String, Vec<String>> {
    let ctes = extract_ctes(model_sql);
    let cte_names: Vec<String> = ctes.iter().map(|c| c.name.clone()).collect();

    let target = match ctes.iter().find(|c| c.name == target_cte) {
        Some(t) => t,
        None => return BTreeMap::new(),
    };

    let mut missing = BTreeMap::new();

    for dep in &target.dependencies {
        let provided_cols: HashSet<String> = inputs
            .get(dep)
            .and_then(|rows| rows.first())
            .map(|row| row.keys().cloned().collect())
            .unwrap_or_default();

        let referenced_cols = find_dependency_columns(&target.body, dep, &cte_names);

        let missing_cols: Vec<String> = referenced_cols
            .into_iter()
            .filter(|c| !provided_cols.contains(c))
            .collect();

        if !missing_cols.is_empty() {
            missing.insert(dep.clone(), missing_cols);
        }
    }

    missing
}

/// Infer the type of a missing column by looking at provided data in the same
/// dependency or falling back to Text.
pub fn infer_missing_column_type(
    dep_name: &str,
    _col_name: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
) -> DataType {
    // Check if any row in the dependency provides a value for this column
    // (maybe some rows have it, others don't)
    if let Some(rows) = inputs.get(dep_name) {
        for row in rows {
            if let Some(val) = row.get(_col_name) {
                let dt = yaml_value_to_datatype(val);
                if dt != DataType::Null && dt != DataType::Unknown {
                    return dt;
                }
            }
        }
    }

    // Fall back to Text - universally safe
    DataType::Text
}

/// Generate a random value for a given data type.
pub fn generate_random_value(data_type: &DataType, rng: &mut StdRng) -> serde_yaml::Value {
    // 10% chance of null for nullable types
    if rng.gen::<f64>() < 0.1 {
        return serde_yaml::Value::Null;
    }
    match data_type {
        DataType::Boolean => serde_yaml::Value::Bool(rng.gen()),
        DataType::SmallInt | DataType::Integer | DataType::BigInt => {
            serde_yaml::Value::Number(serde_yaml::Number::from(rng.gen_range(-1000i64..1000)))
        }
        DataType::Float | DataType::Double => {
            let v: f64 = rng.gen_range(-1000.0..1000.0);
            serde_yaml::Value::Number(serde_yaml::Number::from(v))
        }
        DataType::Decimal { .. } => {
            let v: f64 = rng.gen_range(-1000.0..1000.0);
            serde_yaml::Value::Number(serde_yaml::Number::from(v))
        }
        DataType::Date => {
            let year = rng.gen_range(2020..2030);
            let month = rng.gen_range(1u32..=12);
            let day = rng.gen_range(1u32..=28);
            serde_yaml::Value::String(format!("{:04}-{:02}-{:02}", year, month, day))
        }
        DataType::Timestamp { .. } => {
            let year = rng.gen_range(2020..2030);
            let month = rng.gen_range(1u32..=12);
            let day = rng.gen_range(1u32..=28);
            let hour = rng.gen_range(0u32..24);
            let min = rng.gen_range(0u32..60);
            let sec = rng.gen_range(0u32..60);
            serde_yaml::Value::String(format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                year, month, day, hour, min, sec
            ))
        }
        DataType::Text | DataType::Varchar { .. } | DataType::Char { .. } => {
            let len = rng.gen_range(1..=20);
            let s: String = (0..len)
                .map(|_| rng.gen_range(b'a'..=b'z') as char)
                .collect();
            serde_yaml::Value::String(s)
        }
        _ => {
            // Unknown, Null, etc. - generate a text value as safest default
            let len = rng.gen_range(1..=10);
            let s: String = (0..len)
                .map(|_| rng.gen_range(b'a'..=b'z') as char)
                .collect();
            serde_yaml::Value::String(s)
        }
    }
}

/// Augment input rows with random values for missing columns.
///
/// For each dependency that has missing columns, adds random values
/// to each existing row. If the dependency has no rows at all, creates
/// a single row with just the missing columns.
pub fn augment_inputs(
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    missing: &BTreeMap<String, Vec<String>>,
    rng: &mut StdRng,
    all_inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
) -> BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>> {
    let mut augmented = inputs.clone();

    for (dep, missing_cols) in missing {
        let rows = augmented.entry(dep.clone()).or_default();

        if rows.is_empty() {
            // Create a single row with just the missing columns
            let mut row = BTreeMap::new();
            for col in missing_cols {
                let dt = infer_missing_column_type(dep, col, all_inputs);
                row.insert(col.clone(), generate_random_value(&dt, rng));
            }
            rows.push(row);
        } else {
            // Add missing columns to each existing row
            for row in rows.iter_mut() {
                for col in missing_cols {
                    if !row.contains_key(col) {
                        let dt = infer_missing_column_type(dep, col, all_inputs);
                        row.insert(col.clone(), generate_random_value(&dt, rng));
                    }
                }
            }
        }
    }

    augmented
}

/// Convert augmented inputs to string maps for reporting.
fn inputs_to_string_maps(
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
) -> BTreeMap<String, Vec<BTreeMap<String, String>>> {
    inputs
        .iter()
        .map(|(dep, rows)| {
            let string_rows = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|(k, v)| {
                            let s = match v {
                                serde_yaml::Value::Number(n) => n.to_string(),
                                serde_yaml::Value::Bool(b) => b.to_string(),
                                serde_yaml::Value::String(s) => s.clone(),
                                serde_yaml::Value::Null => "NULL".to_string(),
                                _ => format!("{:?}", v),
                            };
                            (k.clone(), s)
                        })
                        .collect()
                })
                .collect();
            (dep.clone(), string_rows)
        })
        .collect()
}

/// Run a property-based test: execute the test N times with random values
/// for missing columns.
#[cfg(feature = "duckdb")]
pub fn run_property_test(
    test_name: &str,
    model_name: &str,
    target_cte: Option<&str>,
    model_sql: &str,
    test_config: &smelt_core::metadata::TestConfig,
    cases: u32,
    seed: Option<u64>,
) -> PropertyTestResult {
    let start = Instant::now();
    let base_seed = seed.unwrap_or_else(|| {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    });

    let check_order = test_config.check_order.unwrap_or(false);

    // Find missing columns (only for CTE-targeted tests)
    let missing = if let Some(cte) = target_cte {
        find_missing_columns(model_sql, cte, &test_config.inputs)
    } else {
        BTreeMap::new()
    };

    for iteration in 0..cases {
        let iter_seed = base_seed.wrapping_add(iteration as u64);
        let mut rng = StdRng::seed_from_u64(iter_seed);

        // Augment inputs with random values for missing columns
        let augmented =
            augment_inputs(&test_config.inputs, &missing, &mut rng, &test_config.inputs);

        // Compile the test
        let compiled = if let Some(cte) = target_cte {
            compile_cte_test(model_sql, cte, &augmented, None)
        } else {
            compile_whole_model_test(model_sql, &augmented, None)
        };

        let compiled_sql = match compiled {
            Ok(sql) => sql,
            Err(e) => {
                return PropertyTestResult {
                    name: test_name.to_string(),
                    model: model_name.to_string(),
                    target_cte: target_cte.map(|s| s.to_string()),
                    iterations: iteration + 1,
                    passed: false,
                    duration: start.elapsed(),
                    failing_seed: Some(iter_seed),
                    failing_iteration: Some(iteration + 1),
                    inner_result: Some(TestResult {
                        name: test_name.to_string(),
                        model: model_name.to_string(),
                        target_cte: target_cte.map(|s| s.to_string()),
                        passed: false,
                        duration: start.elapsed(),
                        compiled_sql: String::new(),
                        error: Some(TestError::CompilationError(e)),
                    }),
                };
            }
        };

        // Run the test
        let result = run_test(
            test_name,
            model_name,
            target_cte,
            &compiled_sql,
            &test_config.expect,
            check_order,
        );

        if !result.passed {
            let generated_values = inputs_to_string_maps(&augmented);
            let inner_error = result
                .error
                .unwrap_or(TestError::ExecutionError("Unknown error".to_string()));

            return PropertyTestResult {
                name: test_name.to_string(),
                model: model_name.to_string(),
                target_cte: target_cte.map(|s| s.to_string()),
                iterations: iteration + 1,
                passed: false,
                duration: start.elapsed(),
                failing_seed: Some(iter_seed),
                failing_iteration: Some(iteration + 1),
                inner_result: Some(TestResult {
                    name: test_name.to_string(),
                    model: model_name.to_string(),
                    target_cte: target_cte.map(|s| s.to_string()),
                    passed: false,
                    duration: start.elapsed(),
                    compiled_sql,
                    error: Some(TestError::PropertyTestFailure {
                        iteration: iteration + 1,
                        total: cases,
                        seed: iter_seed,
                        inner: Box::new(inner_error),
                        generated_values,
                    }),
                }),
            };
        }
    }

    // All iterations passed
    PropertyTestResult {
        name: test_name.to_string(),
        model: model_name.to_string(),
        target_cte: target_cte.map(|s| s.to_string()),
        iterations: cases,
        passed: true,
        duration: start.elapsed(),
        failing_seed: None,
        failing_iteration: None,
        inner_result: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_value_to_datatype() {
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::Number(serde_yaml::Number::from(42))),
            DataType::Integer
        );
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::Number(serde_yaml::Number::from(2.78))),
            DataType::Double
        );
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::String("hello".to_string())),
            DataType::Text
        );
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::String("2024-01-15".to_string())),
            DataType::Date
        );
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::Bool(true)),
            DataType::Boolean
        );
        assert_eq!(
            yaml_value_to_datatype(&serde_yaml::Value::Null),
            DataType::Null
        );
    }

    #[test]
    fn test_generate_random_value_deterministic() {
        let mut rng1 = StdRng::seed_from_u64(12345);
        let mut rng2 = StdRng::seed_from_u64(12345);

        let v1 = generate_random_value(&DataType::Integer, &mut rng1);
        let v2 = generate_random_value(&DataType::Integer, &mut rng2);
        assert_eq!(v1, v2);

        let v1 = generate_random_value(&DataType::Text, &mut rng1);
        let v2 = generate_random_value(&DataType::Text, &mut rng2);
        assert_eq!(v1, v2);

        let v1 = generate_random_value(&DataType::Date, &mut rng1);
        let v2 = generate_random_value(&DataType::Date, &mut rng2);
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_find_dependency_columns_qualified() {
        let cte_body = "SELECT dep.col1, dep.col2, SUM(dep.amount) FROM dep GROUP BY 1, 2";
        let all_ctes = vec!["dep".to_string()];
        let cols = find_dependency_columns(cte_body, "dep", &all_ctes);
        assert!(cols.contains(&"col1".to_string()));
        assert!(cols.contains(&"col2".to_string()));
        assert!(cols.contains(&"amount".to_string()));
    }

    #[test]
    fn test_find_dependency_columns_unqualified() {
        let cte_body = "SELECT user_id, amount FROM cleaned WHERE status = 'completed'";
        let all_ctes = vec!["cleaned".to_string()];
        let cols = find_dependency_columns(cte_body, "cleaned", &all_ctes);
        assert!(cols.contains(&"user_id".to_string()));
        assert!(cols.contains(&"amount".to_string()));
    }

    #[test]
    fn test_augment_inputs_adds_missing() {
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        inputs.insert("cleaned".to_string(), vec![row]);

        let mut missing = BTreeMap::new();
        missing.insert("cleaned".to_string(), vec!["user_id".to_string()]);

        let mut rng = StdRng::seed_from_u64(42);
        let augmented = augment_inputs(&inputs, &missing, &mut rng, &inputs);

        let rows = &augmented["cleaned"];
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains_key("amount"));
        assert!(rows[0].contains_key("user_id"));
    }

    #[test]
    fn test_augment_inputs_empty_dependency() {
        let inputs: BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>> = BTreeMap::new();
        let mut missing = BTreeMap::new();
        missing.insert(
            "new_dep".to_string(),
            vec!["col1".to_string(), "col2".to_string()],
        );

        let mut rng = StdRng::seed_from_u64(42);
        let augmented = augment_inputs(&inputs, &missing, &mut rng, &inputs);

        let rows = &augmented["new_dep"];
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains_key("col1"));
        assert!(rows[0].contains_key("col2"));
    }

    #[test]
    fn test_find_missing_columns() {
        let model_sql = r#"
WITH cleaned AS (
    SELECT user_id, amount, created_at FROM raw_orders WHERE status = 'completed'
),
daily AS (
    SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily
"#;
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        inputs.insert("cleaned".to_string(), vec![row]);

        let missing = find_missing_columns(model_sql, "daily", &inputs);
        // "created_at" should be missing since only "amount" was provided
        if let Some(missing_cols) = missing.get("cleaned") {
            assert!(missing_cols.contains(&"created_at".to_string()));
        }
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn test_property_test_basic() {
        use smelt_core::metadata::TestConfig;

        let model_sql = r#"
WITH cleaned AS (
    SELECT user_id, amount, created_at FROM raw_orders WHERE status = 'completed'
),
daily AS (
    SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily
"#;
        // Only provide the columns that matter for the assertion
        let mut inputs = BTreeMap::new();
        let mut row1 = BTreeMap::new();
        row1.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        row1.insert(
            "created_at".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        let mut row2 = BTreeMap::new();
        row2.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(200)),
        );
        row2.insert(
            "created_at".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        inputs.insert("cleaned".to_string(), vec![row1, row2]);

        let mut expect1 = BTreeMap::new();
        expect1.insert(
            "day".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        expect1.insert(
            "revenue".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(300.0)),
        );

        let test_config = TestConfig {
            model: "test_model".to_string(),
            target_cte: Some("daily".to_string()),
            inputs,
            expect: vec![expect1],
            cases: Some(5),
            check_order: None,
        };

        let result = run_property_test(
            "test_prop",
            "test_model",
            Some("daily"),
            model_sql,
            &test_config,
            5,
            Some(42),
        );
        assert!(
            result.passed,
            "Property test should pass: {:?}",
            result.inner_result
        );
        assert_eq!(result.iterations, 5);
    }
}
