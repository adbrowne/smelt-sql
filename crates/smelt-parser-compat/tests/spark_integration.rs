//! Spark SQL integration tests via Docker
//!
//! These tests validate SQL by running `EXPLAIN <sql>` against a real Spark SQL instance
//! inside the `apache/spark` Docker container.
//!
//! All tests are `#[ignore]` — run with:
//! ```sh
//! cargo test -p smelt-parser-compat --test spark_integration -- --ignored
//! ```
//!
//! Requires Docker to be available.

use smelt_dialect::{BackendCapabilities, PrintContext, SqlDialect};
use std::process::Command;

/// Rewrite SQL for Spark dialect using smelt-dialect's rewrite system
fn rewrite_for_spark(sql: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let dialect = SqlDialect::SparkSQL;
    let capabilities = BackendCapabilities::spark();
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: &capabilities,
        schema: "default",
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

/// Check if Docker is available and the Spark image can be used
fn spark_docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a SQL statement through Spark's EXPLAIN to validate parsing
/// Returns Ok(()) if Spark successfully parses the SQL, Err with details otherwise.
fn spark_explain(sql: &str) -> Result<(), String> {
    let explain_sql = format!("EXPLAIN {}", sql);
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "apache/spark:latest",
            "/opt/spark/bin/spark-sql",
            "--conf",
            "spark.sql.ansi.enabled=true",
            "-e",
            &explain_sql,
        ])
        .output()
        .map_err(|e| format!("Failed to run docker: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Spark SQL EXPLAIN failed:\n{}", stderr))
    }
}

#[test]
#[ignore]
fn test_spark_docker_available() {
    assert!(
        spark_docker_available(),
        "Docker is not available — Spark integration tests require Docker"
    );
}

#[test]
#[ignore]
fn test_spark_basic_select() {
    // Use a built-in table to avoid needing data setup
    let result = spark_explain("SELECT 1 AS id, 'hello' AS name");
    assert!(result.is_ok(), "Basic SELECT failed: {:?}", result.err());
}

#[test]
#[ignore]
fn test_spark_qualify() {
    // Spark doesn't support QUALIFY natively — smelt-dialect rewrites it to a subquery
    let sql =
        "SELECT * FROM (SELECT 1 AS id, 'a' AS name) t QUALIFY ROW_NUMBER() OVER (ORDER BY id) = 1";
    let rewritten = rewrite_for_spark(sql);
    assert!(
        !rewritten.contains("QUALIFY"),
        "QUALIFY should be rewritten for Spark, got: {}",
        rewritten
    );
    let result = spark_explain(&rewritten);
    assert!(
        result.is_ok(),
        "Rewritten QUALIFY should be valid in Spark: {:?}\nRewritten SQL: {}",
        result.err(),
        rewritten
    );
}

#[test]
#[ignore]
fn test_spark_lambda_transform() {
    let sql = "SELECT TRANSFORM(ARRAY(1, 2, 3), x -> x + 1)";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "TRANSFORM with lambda should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_lambda_aggregate() {
    let sql = "SELECT AGGREGATE(ARRAY(1, 2, 3), 0, (acc, x) -> acc + x)";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "AGGREGATE with lambda should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_pivot() {
    // PIVOT requires a real table or subquery
    let sql =
        "SELECT * FROM (SELECT 'a' AS cat, 1 AS val) t PIVOT (SUM(val) FOR cat IN ('a', 'b'))";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "PIVOT should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_unpivot() {
    let sql = "SELECT * FROM (SELECT 1 AS q1, 2 AS q2) t UNPIVOT (val FOR quarter IN (q1, q2))";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "UNPIVOT should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_array_subscript() {
    let sql = "SELECT ARRAY(1, 2, 3)[0]";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "Array subscript should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_window_function() {
    let sql = "SELECT ROW_NUMBER() OVER (ORDER BY 1)";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "Window function should be valid in Spark: {:?}",
        result.err()
    );
}

#[test]
#[ignore]
fn test_spark_cte() {
    let sql = "WITH t AS (SELECT 1 AS id) SELECT * FROM t";
    let result = spark_explain(sql);
    assert!(
        result.is_ok(),
        "CTE should be valid in Spark: {:?}",
        result.err()
    );
}
