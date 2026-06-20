//! Spark SQL type oracle — executes SQL against a persistent Spark SQL session
//! running inside a Docker container.
//!
//! Instead of spawning a new JVM per query (which takes ~3-5s each), we keep a
//! long-lived `spark-sql` process with stdin/stdout pipes and feed queries through it.

use super::duckdb_oracle::TypeOracle;
use smelt_types::DataType;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Spark-backed oracle using a persistent `spark-sql` process inside a Docker container.
pub struct SparkOracle {
    inner: Mutex<SparkSession>,
}

struct SparkSession {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

impl SparkOracle {
    pub fn new(container_id: &str) -> Self {
        let mut child = Command::new("docker")
            .args(["exec", "-i", container_id, "/opt/spark/bin/spark-sql"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start spark-sql process");

        let stdin = child.stdin.take().expect("failed to get stdin");
        let stdout = child.stdout.take().expect("failed to get stdout");
        let reader = BufReader::new(stdout);

        let oracle = Self {
            inner: Mutex::new(SparkSession {
                child,
                stdin,
                reader,
            }),
        };

        // Wait for Spark to be ready by running a trivial query
        // and draining all startup output until we see our sentinel result
        oracle.warmup();
        oracle
    }

    fn warmup(&self) {
        let mut session = self.inner.lock().unwrap();
        // Send a trivial query with a known sentinel value
        writeln!(session.stdin, "SELECT 'SPARK_READY';").unwrap();
        session.stdin.flush().unwrap();

        // Drain lines until we see the sentinel
        let mut line = String::new();
        loop {
            line.clear();
            match session.reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.contains("SPARK_READY") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }
}

impl Drop for SparkOracle {
    fn drop(&mut self) {
        if let Ok(mut session) = self.inner.lock() {
            let _ = writeln!(session.stdin, "EXIT;");
            let _ = session.stdin.flush();
            let _ = session.child.wait();
        }
    }
}

impl TypeOracle for SparkOracle {
    fn query_types(&self, sql: &str) -> Result<Vec<(String, DataType)>, String> {
        let mut session = self.inner.lock().map_err(|e| format!("lock: {e}"))?;

        // Use a unique sentinel so we know when output for this query ends
        let sentinel = format!(
            "__SMELT_SENTINEL_{}_{}",
            std::process::id(),
            CALL_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let describe_sql = format!("DESCRIBE QUERY {sql}");

        // Send the DESCRIBE QUERY, then a sentinel query
        writeln!(session.stdin, "{describe_sql};").map_err(|e| format!("write describe: {e}"))?;
        writeln!(session.stdin, "SELECT '{sentinel}';")
            .map_err(|e| format!("write sentinel: {e}"))?;
        session.stdin.flush().map_err(|e| format!("flush: {e}"))?;

        // Collect lines until we see the sentinel
        let mut lines = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            match session.reader.read_line(&mut line) {
                Ok(0) => return Err("spark-sql EOF".into()),
                Ok(_) => {
                    if line.contains(&sentinel) {
                        break;
                    }
                    lines.push(line.clone());
                }
                Err(e) => return Err(format!("read: {e}")),
            }
        }

        let output = lines.join("");

        // Check for errors in the output
        if output.contains("AnalysisException")
            || output.contains("ParseException")
            || output.contains("Error in query")
        {
            return Err("spark-sql error in output".to_string());
        }

        parse_describe_output(&output)
    }
}

/// Parse Spark's tab-separated `DESCRIBE QUERY` output.
///
/// Each data line has: `col_name\tdata_type\tcomment`
fn parse_describe_output(output: &str) -> Result<Vec<(String, DataType)>, String> {
    let mut result = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        // Skip empty lines and Spark log/header lines
        if line.is_empty() || line.starts_with('#') || line.starts_with("col_name") {
            continue;
        }
        // Skip separator lines (e.g., "---...---")
        if line.chars().all(|c| c == '-' || c == ' ' || c == '\t') {
            continue;
        }
        // Skip spark-sql prompt lines
        if line.starts_with("spark-sql") {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let col_name = parts[0].trim().to_string();
        let type_str = parts[1].trim();
        let dt = spark_type_to_smelt(type_str);
        result.push((col_name, dt));
    }
    if result.is_empty() {
        return Err("no columns parsed from DESCRIBE QUERY output".into());
    }
    Ok(result)
}

/// Map a Spark SQL type name to a smelt `DataType`.
fn spark_type_to_smelt(type_str: &str) -> DataType {
    let lower_owned = type_str.to_lowercase();
    let lower = lower_owned.trim();

    // Handle parameterized types first
    if let Some(inner) = lower.strip_prefix("decimal(") {
        if let Some(inner) = inner.strip_suffix(')') {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 2 {
                if let (Ok(p), Ok(s)) =
                    (parts[0].trim().parse::<u8>(), parts[1].trim().parse::<u8>())
                {
                    return DataType::Decimal {
                        precision: p,
                        scale: s,
                    };
                }
            }
        }
    }

    match lower {
        "boolean" => DataType::Boolean,
        "tinyint" | "byte" => DataType::SmallInt,
        "smallint" | "short" => DataType::SmallInt,
        "int" | "integer" => DataType::Integer,
        "bigint" | "long" => DataType::BigInt,
        "float" | "real" => DataType::Float,
        "double" => DataType::Double,
        "string" | "varchar" => DataType::Varchar { max_length: None },
        "date" => DataType::Date,
        "timestamp" | "timestamp_ntz" => DataType::Timestamp {
            with_timezone: false,
        },
        "timestamp_ltz" => DataType::Timestamp {
            with_timezone: true,
        },
        "binary" => DataType::Blob,
        "decimal" => DataType::Decimal {
            precision: 10,
            scale: 0,
        },
        "interval" => DataType::Interval,
        "void" | "null" => DataType::Null,
        _ => DataType::unknown_dynamic(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_types() {
        assert_eq!(spark_type_to_smelt("int"), DataType::Integer);
        assert_eq!(spark_type_to_smelt("bigint"), DataType::BigInt);
        assert_eq!(spark_type_to_smelt("double"), DataType::Double);
        assert_eq!(
            spark_type_to_smelt("string"),
            DataType::Varchar { max_length: None }
        );
        assert_eq!(spark_type_to_smelt("boolean"), DataType::Boolean);
        assert_eq!(spark_type_to_smelt("date"), DataType::Date);
        assert_eq!(spark_type_to_smelt("float"), DataType::Float);
        assert_eq!(
            spark_type_to_smelt("timestamp"),
            DataType::Timestamp {
                with_timezone: false
            }
        );
    }

    #[test]
    fn parse_decimal_with_params() {
        assert_eq!(
            spark_type_to_smelt("decimal(10,2)"),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
        assert_eq!(
            spark_type_to_smelt("decimal(38, 10)"),
            DataType::Decimal {
                precision: 38,
                scale: 10
            }
        );
    }

    #[test]
    fn parse_describe_output_basic() {
        let output = "col_name\tdata_type\tcomment\nexpr_0\tint\t\nexpr_1\tstring\t\n";
        let result = parse_describe_output(output).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "expr_0");
        assert_eq!(result[0].1, DataType::Integer);
        assert_eq!(result[1].0, "expr_1");
        assert_eq!(result[1].1, DataType::Varchar { max_length: None });
    }

    #[test]
    fn parse_describe_output_skips_noise() {
        let output = "\n# some log line\ncol_name\tdata_type\tcomment\nx\tbigint\t\n";
        let result = parse_describe_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, DataType::BigInt);
    }
}
