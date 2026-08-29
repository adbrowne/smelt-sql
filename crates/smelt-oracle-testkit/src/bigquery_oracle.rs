//! BigQuery type oracle — asks a live BigQuery warehouse for the output schema
//! of a SELECT via a *dry run*, over a persistent Python subprocess.
//!
//! Two shapes are load-bearing here:
//!
//! * **Dry run, not execution.** A dry-run job carries the full output schema,
//!   bills zero bytes, reads no table, and still rejects invalid SQL. The
//!   oracle therefore learns a query's types without materialising anything.
//! * **One subprocess for the run.** Each dry run costs a network round trip
//!   (~440ms), and a fresh interpreter plus BigQuery client per query would
//!   cost far more, so a single `python -m smelt.bigquery_type_oracle` child
//!   is kept alive and driven with one JSON line in, one JSON line out. That
//!   is simpler than `spark_oracle.rs`'s sentinel framing: the reply is
//!   exactly one line, so there is nothing to frame.

#![allow(dead_code)]

use crate::duckdb_oracle::TypeOracle;
use crate::value::{Cell, ValueOracle};
use serde::Deserialize;
use smelt_types::DataType;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// One column (or struct member) as BigQuery reports it.
///
/// The names are BigQuery's *legacy* ones — `INT64` reports as `INTEGER`,
/// `FLOAT64` as `FLOAT`, `STRUCT` as `RECORD` — and array-ness lives in
/// `mode == "REPEATED"`, never in the type name.
#[derive(Debug, Clone, Deserialize)]
pub struct BqField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub fields: Vec<BqField>,
}

#[derive(Debug, Deserialize)]
struct BqReply {
    #[serde(default)]
    columns: Option<Vec<BqField>>,
    #[serde(default)]
    error: Option<String>,
}

/// One tagged cell of an `exec` reply.
///
/// Every value crosses the protocol as a *string*: a JSON number would round
/// an INT64 past 2^53 and any NUMERIC, and a silently rounded value read as
/// agreement is the failure the value leg exists to catch.
#[derive(Debug, Deserialize)]
struct BqCell {
    t: String,
    #[serde(default)]
    v: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BqRowsReply {
    #[serde(default)]
    rows: Option<Vec<Vec<BqCell>>>,
    #[serde(default)]
    error: Option<String>,
}

/// Read an environment variable, treating "set but empty" as unset — an empty
/// token is not a credential.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// BigQuery-backed oracle over a persistent `smelt.bigquery_type_oracle` child.
pub struct BigQueryOracle {
    inner: Mutex<OracleProcess>,
}

struct OracleProcess {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

impl BigQueryOracle {
    /// Build the oracle from the environment, or return `None` if BigQuery is
    /// not reachable from this checkout.
    ///
    /// The absence of `SMELT_BQ_ACCESS_TOKEN` is the local gate: with no token
    /// the BigQuery leg of the suite is simply not present and the suite is
    /// green. A short-lived token that only a human can mint is the deliberate
    /// cost of reaching a real warehouse from a test.
    pub fn from_env() -> Option<Self> {
        let _project = non_empty_env("SMELT_BQ_PROJECT")?;
        let _token = non_empty_env("SMELT_BQ_ACCESS_TOKEN")?;

        // `scripts/bigquery-env.sh` puts the client venv's site-packages and
        // the repo's `python/` on PYTHONPATH, so a plain `python3` imports the
        // adapter — the same mechanism the PyO3 backend relies on.
        let python = std::env::var("SMELT_BQ_PYTHON").unwrap_or_else(|_| "python3".to_string());

        let mut child = Command::new(&python)
            .args(["-m", "smelt.bigquery_type_oracle"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr is inherited on purpose: the child's diagnostics belong in
            // the test output, where a misconfigured venv is visible.
            .spawn()
            .ok()?;

        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;

        Some(Self {
            inner: Mutex::new(OracleProcess {
                child,
                stdin,
                reader: BufReader::new(stdout),
            }),
        })
    }
}

impl Drop for BigQueryOracle {
    fn drop(&mut self) {
        if let Ok(mut process) = self.inner.lock() {
            // Closing stdin ends the child's `for line in sys.stdin` loop, so
            // it exits on its own rather than being killed mid-request.
            let _ = process.stdin.flush();
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }
}

impl TypeOracle for BigQueryOracle {
    fn query_types(&self, sql: &str) -> Result<Vec<(String, DataType)>, String> {
        // No lock here: `round_trip` takes it. Taking it twice would deadlock.
        let request = serde_json::json!({ "sql": sql }).to_string();
        parse_reply(&self.round_trip(&request)?)
    }
}

impl BigQueryOracle {
    /// Send one protocol request line and read back the single reply line.
    fn round_trip(&self, request: &str) -> Result<String, String> {
        let mut process = self.inner.lock().map_err(|e| format!("lock: {e}"))?;
        writeln!(process.stdin, "{request}").map_err(|e| format!("write request: {e}"))?;
        process.stdin.flush().map_err(|e| format!("flush: {e}"))?;

        let mut line = String::new();
        match process.reader.read_line(&mut line) {
            Ok(0) => Err("bigquery type oracle exited".into()),
            Ok(_) => Ok(line),
            Err(e) => Err(format!("read reply: {e}")),
        }
    }
}

impl ValueOracle for BigQueryOracle {
    /// Really execute `sql` — this bills the warehouse, unlike `query_types`'s
    /// dry run. Only the value leg of the audit calls it.
    fn execute_rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>, String> {
        let request = serde_json::json!({ "exec": sql }).to_string();
        let line = self.round_trip(&request)?;
        parse_rows_reply(&line)
    }
}

/// Decode one `exec` reply line into typed cells.
fn parse_rows_reply(line: &str) -> Result<Vec<Vec<Cell>>, String> {
    let reply: BqRowsReply =
        serde_json::from_str(line.trim()).map_err(|e| format!("bad reply {line:?}: {e}"))?;
    if let Some(error) = reply.error {
        return Err(error);
    }
    let rows = reply
        .rows
        .ok_or_else(|| format!("reply had neither rows nor error: {line:?}"))?;
    rows.iter()
        .map(|row| row.iter().map(decode_tagged_cell).collect())
        .collect()
}

/// Decode one `{"t": …, "v": …}` pair.
///
/// An unrecognised tag is an error, not a `Text` fallback: the protocol is
/// ours on both ends, so a tag we do not know means the two halves have
/// drifted, and quietly stringifying it would hide that behind a value
/// divergence report.
fn decode_tagged_cell(cell: &BqCell) -> Result<Cell, String> {
    if cell.t == "null" {
        return Ok(Cell::Null);
    }
    let v = cell
        .v
        .as_deref()
        .ok_or_else(|| format!("tagged cell `{}` carried no value", cell.t))?;
    let parsed = |e: &str| format!("cell tagged `{}` holds {v:?}, which is not {e}", cell.t);
    match cell.t.as_str() {
        "int" => v
            .parse::<i128>()
            .map(Cell::Int)
            .map_err(|_| parsed("an integer")),
        "float" => v
            .parse::<f64>()
            .map(Cell::Float)
            .map_err(|_| parsed("a float")),
        "bool" => match v {
            "true" => Ok(Cell::Bool(true)),
            "false" => Ok(Cell::Bool(false)),
            _ => Err(parsed("a boolean")),
        },
        "text" => Ok(Cell::Text(v.to_string())),
        "decimal" => decode_decimal(v).ok_or_else(|| parsed("a decimal")),
        "date" => Ok(Cell::Date(v.to_string())),
        "timestamp" => Ok(Cell::Timestamp(v.to_string())),
        other => Err(format!(
            "unknown cell tag `{other}` — the Rust and Python halves of the protocol have drifted"
        )),
    }
}

/// Parse a plain decimal rendering into an unscaled/scale pair.
fn decode_decimal(text: &str) -> Option<Cell> {
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text, ""));
    if frac_part.chars().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    Some(Cell::Decimal {
        unscaled: format!("{int_part}{frac_part}").parse::<i128>().ok()?,
        scale: frac_part.len() as u32,
    })
}

/// Turn one protocol reply line into columns, or into the warehouse's refusal.
fn parse_reply(line: &str) -> Result<Vec<(String, DataType)>, String> {
    let reply: BqReply =
        serde_json::from_str(line.trim()).map_err(|e| format!("bad reply {line:?}: {e}"))?;

    if let Some(error) = reply.error {
        return Err(error);
    }
    let columns = reply
        .columns
        .ok_or_else(|| format!("reply had neither columns nor error: {line:?}"))?;

    Ok(columns
        .iter()
        .map(|field| {
            (
                field.name.clone(),
                bigquery_type_to_smelt(&field.field_type, &field.mode, &field.fields),
            )
        })
        .collect())
}

/// Map a BigQuery reported `(field_type, mode, fields)` triple to a smelt
/// `DataType`.
///
/// The triple is what a query output schema actually carries, so all three
/// parts matter: the type name is the *legacy* one, the element/struct shape
/// is in `fields`, and array-ness is in `mode`.
fn bigquery_type_to_smelt(field_type: &str, mode: &str, fields: &[BqField]) -> DataType {
    let element = match field_type.to_ascii_uppercase().as_str() {
        // BigQuery has exactly one integer width (INT64) and one float width
        // (FLOAT64). Reporting INTEGER as `Integer` would assert a 32-bit width
        // the warehouse does not have.
        "INTEGER" | "INT64" => DataType::BigInt,
        "FLOAT" | "FLOAT64" => DataType::Double,
        "BOOLEAN" | "BOOL" => DataType::Boolean,
        "STRING" => DataType::Varchar { max_length: None },
        "BYTES" => DataType::Blob,
        "DATE" => DataType::Date,
        "TIME" => DataType::Time,
        "INTERVAL" => DataType::Interval,
        // TIMESTAMP is an absolute instant; DATETIME is the naive wall-clock
        // one. The pairing is the reverse of several other engines', so it is
        // spelled out rather than inferred from the name.
        "TIMESTAMP" => DataType::Timestamp {
            with_timezone: true,
        },
        "DATETIME" => DataType::Timestamp {
            with_timezone: false,
        },
        // precision 0 / scale 0 means **precision not reported by BigQuery**,
        // not precision zero: a query output schema carries `precision: None`
        // and `scale: None` for NUMERIC and BIGNUMERIC alike, so there is no
        // width to report. Comparison against smelt's inferred decimal width is
        // handled by a registered divergence, not by pretending to a number.
        "NUMERIC" | "BIGNUMERIC" => DataType::Decimal {
            precision: 0,
            scale: 0,
        },
        // Matches how the DuckDB oracle surfaces JSON (`json_type_check` there),
        // so the two oracles answer the same question the same way.
        "JSON" => DataType::Varchar { max_length: None },
        // A struct: the member types live only in `fields`, since the parent
        // reports nothing but `RECORD`.
        "RECORD" | "STRUCT" => DataType::Struct(
            fields
                .iter()
                .map(|f| {
                    (
                        f.name.clone(),
                        bigquery_type_to_smelt(&f.field_type, &f.mode, &f.fields),
                    )
                })
                .collect(),
        ),
        _ => DataType::unknown_dynamic(),
    };

    // The wrap happens last, and applies to RECORD too: `[STRUCT(1 AS a)]`
    // reports as ('RECORD','REPEATED',…), i.e. an array of structs.
    if mode.eq_ignore_ascii_case("REPEATED") {
        DataType::Array(Box::new(element))
    } else {
        element
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(field_type: &str) -> DataType {
        bigquery_type_to_smelt(field_type, "NULLABLE", &[])
    }

    fn field(name: &str, field_type: &str) -> BqField {
        BqField {
            name: name.to_string(),
            field_type: field_type.to_string(),
            mode: "NULLABLE".to_string(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn scalar_types() {
        // `CAST(1 AS INT64)` reports as INTEGER; BigQuery has exactly one
        // integer width, so BigInt is the only honest mapping.
        assert_eq!(map("INTEGER"), DataType::BigInt);
        // `CAST(1.5 AS FLOAT64)` reports as FLOAT; BigQuery has one float width.
        assert_eq!(map("FLOAT"), DataType::Double);
        assert_eq!(map("BOOLEAN"), DataType::Boolean);
        assert_eq!(map("STRING"), DataType::Varchar { max_length: None });
        assert_eq!(map("BYTES"), DataType::Blob);
        assert_eq!(map("DATE"), DataType::Date);
        assert_eq!(map("TIME"), DataType::Time);
        assert_eq!(map("INTERVAL"), DataType::Interval);
        assert_eq!(map("GEOGRAPHY"), DataType::unknown_dynamic());
    }

    #[test]
    fn timestamp_vs_datetime() {
        // BigQuery's TIMESTAMP is an absolute instant; its DATETIME is naive.
        assert_eq!(
            map("TIMESTAMP"),
            DataType::Timestamp {
                with_timezone: true
            }
        );
        assert_eq!(
            map("DATETIME"),
            DataType::Timestamp {
                with_timezone: false
            }
        );
    }

    #[test]
    fn decimals_report_no_precision() {
        // BigQuery reports precision/scale as None on a query output schema, so
        // 0/0 stands for "not reported", never for precision zero.
        assert_eq!(
            map("NUMERIC"),
            DataType::Decimal {
                precision: 0,
                scale: 0
            }
        );
        assert_eq!(
            map("BIGNUMERIC"),
            DataType::Decimal {
                precision: 0,
                scale: 0
            }
        );
    }

    #[test]
    fn json_matches_duckdb_oracle() {
        // The DuckDB oracle surfaces JSON as Varchar (see `json_type_check`);
        // the two oracles must agree or every JSON case reads as a divergence.
        assert_eq!(map("JSON"), DataType::Varchar { max_length: None });
    }

    #[test]
    fn repeated_mode_is_the_array() {
        // `[1,2,3]` reports as ('INTEGER','REPEATED') — array-ness is in the
        // mode, never in the type name.
        assert_eq!(
            bigquery_type_to_smelt("INTEGER", "REPEATED", &[]),
            DataType::Array(Box::new(DataType::BigInt))
        );
        assert_eq!(
            bigquery_type_to_smelt("STRING", "REPEATED", &[]),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );
    }

    #[test]
    fn record_becomes_struct() {
        // `STRUCT(1 AS a, 'x' AS b)` → ('RECORD','NULLABLE',[a:INTEGER, b:STRING])
        let fields = vec![field("a", "INTEGER"), field("b", "STRING")];
        assert_eq!(
            bigquery_type_to_smelt("RECORD", "NULLABLE", &fields),
            DataType::Struct(vec![
                ("a".to_string(), DataType::BigInt),
                ("b".to_string(), DataType::Varchar { max_length: None }),
            ])
        );
    }

    #[test]
    fn repeated_record_is_array_of_struct() {
        // `[STRUCT(1 AS a)]` → ('RECORD','REPEATED',[a:INTEGER]): the struct is
        // built first, then wrapped.
        let fields = vec![field("a", "INTEGER")];
        assert_eq!(
            bigquery_type_to_smelt("RECORD", "REPEATED", &fields),
            DataType::Array(Box::new(DataType::Struct(vec![(
                "a".to_string(),
                DataType::BigInt
            )])))
        );
    }

    #[test]
    fn nested_record_recurses() {
        let inner = BqField {
            name: "inner".to_string(),
            field_type: "RECORD".to_string(),
            mode: "NULLABLE".to_string(),
            fields: vec![field("x", "INTEGER")],
        };
        assert_eq!(
            bigquery_type_to_smelt("RECORD", "NULLABLE", &[inner]),
            DataType::Struct(vec![(
                "inner".to_string(),
                DataType::Struct(vec![("x".to_string(), DataType::BigInt)])
            )])
        );
    }

    #[test]
    fn unknown_type_name_is_unknown() {
        assert_eq!(map("SOMETHING_NEW"), DataType::unknown_dynamic());
    }

    #[test]
    fn parse_reply_columns() {
        let line = r#"{"columns":[{"name":"c","type":"INTEGER","mode":"NULLABLE","fields":[]}]}"#;
        let parsed = parse_reply(line).unwrap();
        assert_eq!(parsed, vec![("c".to_string(), DataType::BigInt)]);
    }

    #[test]
    fn parse_reply_error_is_err() {
        let line = r#"{"error":"400 Function not found: nosuchfunction"}"#;
        let err = parse_reply(line).unwrap_err();
        assert!(err.contains("nosuchfunction"));
    }

    #[test]
    fn tagged_cells_decode_to_typed_cells() {
        let reply = r#"{"rows": [[{"t":"int","v":"5"},{"t":"null"},{"t":"decimal","v":"1.50"},{"t":"bool","v":"true"},{"t":"text","v":"hi"}]]}"#;
        let rows = parse_rows_reply(reply).expect("decode");
        assert_eq!(
            rows,
            vec![vec![
                Cell::Int(5),
                Cell::Null,
                Cell::Decimal {
                    unscaled: 150,
                    scale: 2
                },
                Cell::Bool(true),
                Cell::Text("hi".into()),
            ]]
        );
    }

    /// The whole reason values cross as strings: a JSON number would round
    /// this to 9007199254740992.
    #[test]
    fn an_int64_past_the_double_range_survives_the_protocol() {
        let reply = r#"{"rows": [[{"t":"int","v":"9007199254740993"}]]}"#;
        let rows = parse_rows_reply(reply).expect("decode");
        assert_eq!(rows[0][0], Cell::Int(9_007_199_254_740_993));
    }

    #[test]
    fn a_refusal_is_an_error_not_zero_rows() {
        let err = parse_rows_reply(r#"{"error": "400 POST ... Syntax error"}"#)
            .expect_err("a refusal must not read as an empty result set");
        assert!(err.contains("Syntax error"));
    }

    /// An unknown tag means the Rust and Python halves have drifted. Falling
    /// back to `Text` would surface that as a value divergence on some
    /// unrelated function instead.
    #[test]
    fn an_unknown_tag_is_a_protocol_error() {
        let err = parse_rows_reply(r#"{"rows": [[{"t":"geography","v":"POINT(0 0)"}]]}"#)
            .expect_err("unknown tags must fail loud");
        assert!(err.contains("drifted"), "{err}");
    }

    /// Live leg. Skips green when no BigQuery token is exported.
    #[test]
    fn the_bigquery_value_oracle_round_trips_against_a_live_warehouse() {
        let Some(oracle) = BigQueryOracle::from_env() else {
            eprintln!("BigQuery not configured — skipping live BigQuery value leg");
            return;
        };
        let rows = oracle.execute_rows("SELECT 2 + 3 AS s").expect("execute");
        assert_eq!(rows, vec![vec![Cell::Int(5)]]);
        let rows = oracle
            .execute_rows("SELECT CAST(NULL AS INT64) AS n")
            .expect("execute");
        assert_eq!(rows, vec![vec![Cell::Null]]);
    }
}
