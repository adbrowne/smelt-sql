//! Trino type oracle — asks a live Trino coordinator for the output schema of
//! a SELECT via `TrinoClient::execute_schema`, which submits the query and
//! follows `nextUri` to completion without decoding any row.
//!
//! The `ValueOracle` impl below is a separate path from the schema leg's:
//! it goes through `TrinoClient::execute_json`, which hands back Trino's own
//! JSON cells undecoded, and `cell_from_trino_json` maps each one against its
//! own declared raw type. This is deliberately not built on
//! `arrow_convert::trino_type_to_arrow` — that converter has no array/
//! varbinary/interval arm, and a decode error there would masquerade as the
//! engine rejecting the probe, the exact confusion the schema leg was
//! designed to avoid.

use crate::arrow_mapping::arrow_to_smelt;
use crate::duckdb_oracle::TypeOracle;
use crate::value::{Cell, ValueOracle};
use smelt_backend_trino::arrow_convert::trino_type_to_arrow;
use smelt_backend_trino::{TrinoClient, TrinoClientConfig};
use smelt_types::DataType;
use std::sync::Mutex;
use tokio::runtime::Runtime;

/// Read an environment variable, treating "set but empty" as unset.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Trino-backed oracle over the `/v1/statement` HTTP client.
///
/// Holds its own current-thread runtime rather than requiring an ambient
/// `#[tokio::test]` context, so it can be driven from ordinary synchronous
/// test functions exactly like the DuckDB and BigQuery oracles.
pub struct TrinoOracle {
    client: TrinoClient,
    runtime: Mutex<Runtime>,
}

impl TrinoOracle {
    /// Build the oracle from the environment, or return `None` if
    /// `SMELT_TRINO_URL` is unset — the local gate that keeps the Trino leg
    /// of the suite green with no tier running.
    pub fn from_env() -> Option<Self> {
        let base_url = non_empty_env("SMELT_TRINO_URL")?;
        let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
        let catalog =
            std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
        let schema =
            std::env::var("SMELT_TRINO_SCHEMA").unwrap_or_else(|_| "smelt_dev".to_string());
        let runtime = Runtime::new().ok()?;

        Some(Self {
            client: TrinoClient::new(TrinoClientConfig {
                base_url,
                user,
                catalog,
                schema,
                password: None,
            }),
            runtime: Mutex::new(runtime),
        })
    }
}

impl TrinoOracle {
    /// Execute `sql` and return the total row count across every returned
    /// batch. Not the `ValueOracle` impl below — this only proves the
    /// fixture executes and yields the right row count, so it decodes
    /// whatever columns `sql` selects via the ordinary `TrinoClient::execute`
    /// path (`arrow_convert`, not `cell_from_trino_json`). Callers must avoid
    /// selecting a type `trino_type_to_arrow` doesn't yet recognise (arrays,
    /// VARBINARY, INTERVAL).
    pub fn row_count(&self, sql: &str) -> Result<usize, String> {
        let runtime = self.runtime.lock().map_err(|e| format!("lock: {e}"))?;
        let batches = runtime
            .block_on(self.client.execute(sql))
            .map_err(|e| e.to_string())?;
        Ok(batches.iter().map(|b| b.num_rows()).sum())
    }
}

impl TypeOracle for TrinoOracle {
    fn query_types(&self, sql: &str) -> Result<Vec<(String, DataType)>, String> {
        let runtime = self.runtime.lock().map_err(|e| format!("lock: {e}"))?;
        let columns = runtime
            .block_on(self.client.execute_schema(sql))
            .map_err(|e| e.to_string())?;

        columns
            .into_iter()
            .map(|(name, raw_type)| {
                let arrow_ty = trino_type_to_arrow(&raw_type).map_err(|e| e.to_string())?;
                Ok((name, arrow_to_smelt(&arrow_ty)))
            })
            .collect()
    }
}

impl ValueOracle for TrinoOracle {
    /// Execute `sql` and decode each cell against the raw type Trino itself
    /// reported for that column, via `execute_json` — never through
    /// `arrow_convert` (see the module doc).
    fn execute_rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>, String> {
        let runtime = self.runtime.lock().map_err(|e| format!("lock: {e}"))?;
        let (columns, rows) = runtime
            .block_on(self.client.execute_json(sql))
            .map_err(|e| e.to_string())?;

        Ok(rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .zip(columns.iter())
                    .map(|(value, (_, raw_type))| cell_from_trino_json(raw_type, &value))
                    .collect()
            })
            .collect())
    }
}

/// Decode one Trino `/v1/statement` JSON cell against the raw column type
/// Trino reported for it (e.g. `"bigint"`, `"decimal(10,2)"`,
/// `"array(integer)"`). The base type name — the part before any `(...)` —
/// selects the decode; parameters (precision/scale, array element type) are
/// read from the same string when needed.
pub fn cell_from_trino_json(raw_type: &str, value: &serde_json::Value) -> Cell {
    if value.is_null() {
        return Cell::Null;
    }
    let base = raw_type.split('(').next().unwrap_or(raw_type).trim();
    match base {
        "tinyint" | "smallint" | "integer" | "bigint" => match value.as_i64() {
            Some(n) => Cell::Int(n as i128),
            None => Cell::Text(value.to_string()),
        },
        // Trino's JSON protocol has no numeric spelling for NaN/Infinity (JSON
        // itself has none), so it serialises those as the strings `"NaN"`,
        // `"Infinity"`, `"-Infinity"` rather than a JSON number. `f64::from_str`
        // parses all three case-insensitively, matching DuckDB's own
        // `Float(NaN)`/`Float(inf)` cells rather than falling to `Text`.
        "real" | "double" => match value.as_f64() {
            Some(f) => Cell::Float(f),
            None => match value.as_str().and_then(|s| s.parse::<f64>().ok()) {
                Some(f) => Cell::Float(f),
                None => Cell::Text(value.to_string()),
            },
        },
        "decimal" => value
            .as_str()
            .and_then(decode_trino_decimal)
            .unwrap_or_else(|| Cell::Text(value.to_string())),
        "boolean" => match value.as_bool() {
            Some(b) => Cell::Bool(b),
            None => Cell::Text(value.to_string()),
        },
        "date" => match value.as_str() {
            Some(s) => Cell::Date(s.to_string()),
            None => Cell::Text(value.to_string()),
        },
        _ if base.starts_with("timestamp") => match value.as_str() {
            Some(s) => Cell::Timestamp(s.to_string()),
            None => Cell::Text(value.to_string()),
        },
        "array" => Cell::Text(format_trino_array(raw_type, value)),
        _ => match value.as_str() {
            Some(s) => Cell::Text(s.to_string()),
            None => Cell::Text(value.to_string()),
        },
    }
}

/// Parse a Trino decimal rendering (e.g. `"1.23"`, `"-1.230"`) into an
/// unscaled/scale pair, matching `decode_decimal_text` in `spark_oracle.rs`.
fn decode_trino_decimal(text: &str) -> Option<Cell> {
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text, ""));
    if frac_part.chars().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    Some(Cell::Decimal {
        unscaled: format!("{int_part}{frac_part}").parse::<i128>().ok()?,
        scale: frac_part.len() as u32,
    })
}

/// Render a Trino `array(...)` JSON value as a bracketed text cell, matching
/// the bracket-and-comma spelling `value.rs`'s `collection_text` normalises
/// so it compares equal to DuckDB's and Spark's own array renderings. The
/// element type comes from `raw_type`'s own `array(<element>)` spelling, so
/// a nested element (e.g. a decimal string) still decodes correctly rather
/// than falling back to the outer `array` arm.
fn format_trino_array(raw_type: &str, value: &serde_json::Value) -> String {
    let element_type = raw_type
        .strip_prefix("array(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("varchar");
    let Some(items) = value.as_array() else {
        return value.to_string();
    };
    let rendered: Vec<String> = items
        .iter()
        .map(|item| cell_display(&cell_from_trino_json(element_type, item)))
        .collect();
    format!("[{}]", rendered.join(", "))
}

/// Plain text spelling of a decoded cell, used only to render an array
/// element inside `format_trino_array` — not a general `Cell` formatter.
fn cell_display(cell: &Cell) -> String {
    match cell {
        Cell::Null => "NULL".to_string(),
        Cell::Int(n) => n.to_string(),
        Cell::Float(f) => f.to_string(),
        Cell::Bool(b) => b.to_string(),
        Cell::Text(s) => s.clone(),
        Cell::Date(s) | Cell::Timestamp(s) => s.clone(),
        Cell::Decimal { unscaled, scale } => {
            if *scale == 0 {
                unscaled.to_string()
            } else {
                let s = unscaled.unsigned_abs().to_string();
                let s = format!("{s:0>width$}", width = *scale as usize + 1);
                let (int_part, frac_part) = s.split_at(s.len() - *scale as usize);
                format!(
                    "{}{int_part}.{frac_part}",
                    if *unscaled < 0 { "-" } else { "" }
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live leg. Skips green when no Trino tier is exported.
    #[test]
    fn trino_oracle_reports_column_types() {
        let Some(oracle) = TrinoOracle::from_env() else {
            eprintln!("SMELT_TRINO_URL unset — skipping trino_oracle_reports_column_types");
            return;
        };
        let types = oracle
            .query_types("SELECT CAST(1 AS BIGINT) AS a, CAST('x' AS VARCHAR) AS b")
            .expect("query_types");
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].0, "a");
        assert_eq!(types[0].1, DataType::BigInt);
        assert_eq!(types[1].0, "b");
        assert_eq!(types[1].1, DataType::Varchar { max_length: None });
    }

    /// Live leg. A syntactically invalid query returns `Err`, so the leg can
    /// distinguish rejection from an empty schema.
    #[test]
    fn trino_oracle_errors_on_a_rejected_query() {
        let Some(oracle) = TrinoOracle::from_env() else {
            eprintln!("SMELT_TRINO_URL unset — skipping trino_oracle_errors_on_a_rejected_query");
            return;
        };
        let err = oracle
            .query_types("SELECT this is not valid sql")
            .expect_err("a rejected query must not read as an empty schema");
        assert!(!err.is_empty());
    }

    /// Offline. Every base type family `cell_from_trino_json` recognises,
    /// including a nested array — the shape that motivated bypassing
    /// `arrow_convert` in the first place.
    #[test]
    fn trino_json_cells_decode_by_declared_type() {
        use serde_json::json;

        assert_eq!(cell_from_trino_json("bigint", &json!(42)), Cell::Int(42));
        assert_eq!(
            cell_from_trino_json("double", &json!(1.5)),
            Cell::Float(1.5)
        );
        assert_eq!(
            cell_from_trino_json("decimal(10,2)", &json!("1.23")),
            Cell::Decimal {
                unscaled: 123,
                scale: 2
            }
        );
        assert_eq!(
            cell_from_trino_json("varchar(10)", &json!("x")),
            Cell::Text("x".to_string())
        );
        assert_eq!(
            cell_from_trino_json("boolean", &json!(true)),
            Cell::Bool(true)
        );
        assert_eq!(
            cell_from_trino_json("date", &json!("2026-01-01")),
            Cell::Date("2026-01-01".to_string())
        );
        assert_eq!(
            cell_from_trino_json("timestamp", &json!("2026-01-01 01:02:03.456")),
            Cell::Timestamp("2026-01-01 01:02:03.456".to_string())
        );
        assert_eq!(cell_from_trino_json("bigint", &json!(null)), Cell::Null);
        assert_eq!(
            cell_from_trino_json("array(integer)", &json!([1, 2, 3])),
            Cell::Text("[1, 2, 3]".to_string())
        );
    }

    /// Live leg. Skips green when no Trino tier is exported.
    #[test]
    fn trino_oracle_executes_rows() {
        let Some(oracle) = TrinoOracle::from_env() else {
            eprintln!("SMELT_TRINO_URL unset — skipping trino_oracle_executes_rows");
            return;
        };
        let rows = oracle
            .execute_rows("SELECT 2 + 3 AS s, CAST(NULL AS BIGINT) AS n")
            .expect("execute_rows");
        assert_eq!(rows, vec![vec![Cell::Int(5), Cell::Null]]);
    }
}
