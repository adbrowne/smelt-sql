//! Spark SQL type oracle — executes SQL against a persistent Spark SQL session
//! running inside a Docker container.
//!
//! Instead of spawning a new JVM per query (which takes ~3-5s each), we keep a
//! long-lived `spark-sql` process with stdin/stdout pipes and feed queries through it.

use crate::duckdb_oracle::TypeOracle;
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
        // `spark-sql` writes query errors to stderr, not stdout, so the stream
        // is merged inside the container. Without this the oracle sees an empty
        // `DESCRIBE QUERY` result and cannot tell a query the engine refused
        // from an oracle that has stopped working — see `query_types`.
        let mut child = Command::new("docker")
            .args([
                "exec",
                "-i",
                container_id,
                "sh",
                "-c",
                "/opt/spark/bin/spark-sql 2>&1",
            ])
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

        interpret_describe_output(&output)
    }
}

/// Turn one `DESCRIBE QUERY` response into column types, or into an error
/// string whose wording tells `classify_oracle_error` whether the engine
/// refused this query (skippable) or the oracle itself is blind (fatal).
///
/// Split out of `query_types` so both branches can be pinned by unit tests
/// without a live Spark session.
fn interpret_describe_output(output: &str) -> Result<Vec<(String, DataType)>, String> {
    // Spark 4 reports a refused query as a named error condition with a
    // trailing SQLSTATE rather than the exception names 3.x used — e.g.
    // `CAST('x' AS VARCHAR)` yields `[DATATYPE_MISSING_SIZE] DataType
    // "VARCHAR" requires a length parameter ... SQLSTATE: 42K01`, which
    // carries none of the three legacy strings. Both spellings are matched so
    // the oracle keeps classifying an engine refusal as a refusal on either
    // version.
    if output.contains("AnalysisException")
        || output.contains("ParseException")
        || output.contains("Error in query")
        || output.contains("SQLSTATE:")
    {
        return Err("spark-sql error in output".to_string());
    }

    match parse_describe_output(output) {
        Ok(columns) => Ok(columns),
        // No column rows came back. Reading the sentinel already proved the
        // session is alive, so this is one of two very different things and the
        // oracle must not conflate them (see `classify_oracle_error`): the
        // engine refused this query and explained itself in prose, or
        // `DESCRIBE QUERY`'s output shape changed and the parser is now blind —
        // which would silently "skip" every case and report green while
        // verifying nothing.
        //
        // The discriminator is the tab. A successful `DESCRIBE QUERY` row is
        // tab-separated (`col_name\tdata_type\tcomment`); Spark's refusal
        // messages are prose with no tabs. So tab-bearing output that failed to
        // parse is a format change (fatal, left unrecognised), while tab-free
        // prose is the engine rejecting this specific SQL. Verified live on
        // Spark 4.0.0: `CUME_DIST() OVER (... ROWS ...)` yields "Window Frame
        // specifiedwindowframe(RowFrame, -1, 1) must match the required frame
        // ..." — a refusal carrying neither an exception name nor a SQLSTATE.
        Err(e) => {
            if diagnostic_lines(output).any(|line| line.contains('\t')) {
                Err(format!("unparseable DESCRIBE QUERY output: {e}"))
            } else if diagnostic_lines(output).next().is_some() {
                Err("spark-sql error in output".to_string())
            } else {
                Err(format!("{e} (no output at all)"))
            }
        }
    }
}

/// The lines of `spark-sql` output that carry information about the statement,
/// with the CLI's own scaffolding removed: the echoed prompt (which repeats the
/// submitted SQL verbatim), blank lines, and the trailing timing line.
///
/// Used to tell an engine refusal from an output-format change; see the call
/// site in `query_types`.
fn diagnostic_lines(output: &str) -> impl Iterator<Item = &str> {
    output.lines().map(str::trim).filter(|line| {
        !line.is_empty() && !line.starts_with("spark-sql") && !line.starts_with("Time taken:")
    })
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

    // ARRAY<element_type> — recurse on the element type.
    if let Some(inner) = lower.strip_prefix("array<") {
        if let Some(inner) = inner.strip_suffix('>') {
            return DataType::Array(Box::new(spark_type_to_smelt(inner)));
        }
    }

    // Interval types report as e.g. "interval", "interval day", "interval year",
    // "interval day to second", "interval year to month" — never just "interval"
    // alone in practice, since Spark always qualifies year-month vs day-time
    // granularity. Match the family by prefix rather than a single exact string.
    if lower.starts_with("interval") {
        return DataType::Interval;
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
    fn parse_interval_variants() {
        // Regression: a local soak run caught `TIMESTAMP - TIMESTAMP` mapping to
        // Unknown(Dynamic) instead of Interval because Spark's `typeof` never
        // reports a bare "interval" — it always qualifies year-month vs
        // day-time granularity (e.g. "interval day to second").
        assert_eq!(spark_type_to_smelt("interval"), DataType::Interval);
        assert_eq!(spark_type_to_smelt("interval day"), DataType::Interval);
        assert_eq!(spark_type_to_smelt("interval year"), DataType::Interval);
        assert_eq!(
            spark_type_to_smelt("interval day to second"),
            DataType::Interval
        );
        assert_eq!(
            spark_type_to_smelt("interval year to month"),
            DataType::Interval
        );
    }

    #[test]
    fn parse_array_type() {
        assert_eq!(
            spark_type_to_smelt("array<boolean>"),
            DataType::Array(Box::new(DataType::Boolean))
        );
        assert_eq!(
            spark_type_to_smelt("array<int>"),
            DataType::Array(Box::new(DataType::Integer))
        );
        assert_eq!(
            spark_type_to_smelt("array<array<int>>"),
            DataType::Array(Box::new(DataType::Array(Box::new(DataType::Integer))))
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

    /// Regression: Spark 4 spells a refused query as a named error condition
    /// with a trailing SQLSTATE, carrying none of the 3.x exception names.
    /// Live on Spark 4.0.0, `CAST('hello' AS VARCHAR)` (no length) produces
    /// exactly this. It must read as a refusal, not a broken oracle.
    #[test]
    fn sqlstate_error_is_a_query_refusal() {
        let output = "[DATATYPE_MISSING_SIZE] DataType \"VARCHAR\" requires a length \
                      parameter, for example \"VARCHAR\"(10). Please specify the length. \
                      SQLSTATE: 42K01\n";
        let err = interpret_describe_output(output).unwrap_err();
        assert_eq!(err, "spark-sql error in output");
    }

    /// Regression for the `prop_type_inference` seed pinned in
    /// `type_property_tests.proptest-regressions`: `CUME_DIST() OVER (... ROWS
    /// ...)` is refused by Spark with prose carrying neither an exception name
    /// nor a SQLSTATE. The tab-free-prose branch is what classifies it.
    #[test]
    fn bare_prose_error_is_a_query_refusal() {
        let output = "spark-sql (default)> DESCRIBE QUERY SELECT CUME_DIST() OVER (ORDER \
                      BY d ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING);\n\
                      Window Frame specifiedwindowframe(RowFrame, -1, 1) must match the \
                      required frame specifiedwindowframe(RangeFrame, unboundedpreceding$(), \
                      currentrow$()).\nTime taken: 0.3 seconds\n";
        let err = interpret_describe_output(output).unwrap_err();
        assert_eq!(err, "spark-sql error in output");
    }

    /// The fail-loud half: output that is still tab-shaped but no longer
    /// parseable means `DESCRIBE QUERY`'s format moved, which would make every
    /// case "skip" and the leg report green while verifying nothing. That must
    /// stay unrecognised so `classify_oracle_error` calls it fatal.
    #[test]
    fn unparseable_tab_shaped_output_is_not_a_refusal() {
        let output = "# col_name\tdata_type\tcomment\n";
        let err = interpret_describe_output(output).unwrap_err();
        assert!(
            err.starts_with("unparseable DESCRIBE QUERY output"),
            "expected a fatal-shaped error, got {err:?}"
        );
        assert_ne!(err, "spark-sql error in output");
    }

    /// Silence is also not a refusal: a session that answers nothing at all is
    /// a dead oracle, not an engine rejecting one query.
    #[test]
    fn empty_output_is_not_a_refusal() {
        let err = interpret_describe_output("spark-sql (default)> \n\n").unwrap_err();
        assert!(err.contains("no output at all"), "got {err:?}");
        assert_ne!(err, "spark-sql error in output");
    }
}
