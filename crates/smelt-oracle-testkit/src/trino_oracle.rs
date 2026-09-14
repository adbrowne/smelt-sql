//! Trino type oracle — asks a live Trino coordinator for the output schema of
//! a SELECT via `TrinoClient::execute_schema`, which submits the query and
//! follows `nextUri` to completion without decoding any row.
//!
//! No `ValueOracle` impl here: the schema leg needs only column metadata, and
//! materialising rows for a value comparison is phase 6's job
//! (`docs/outcomes/20260913-trino-emission/outcome.md`).

use crate::arrow_mapping::arrow_to_smelt;
use crate::duckdb_oracle::TypeOracle;
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
    /// batch. Not a `ValueOracle` impl (that is phase 6's job) — this only
    /// proves the fixture executes and yields the right row count, so it
    /// decodes whatever columns `sql` selects via the ordinary
    /// `TrinoClient::execute` path. Callers must avoid selecting a type
    /// `trino_type_to_arrow` doesn't yet recognise (arrays, VARBINARY,
    /// INTERVAL).
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
}
