//! The `/v1/statement` + `nextUri` paging HTTP client.

use arrow::array::RecordBatch;
use smelt_backend::BackendError;

use crate::arrow_convert::rows_to_record_batch;
use crate::config::TrinoClientConfig;
use crate::error::{map_transport_error, map_trino_error};
use crate::protocol::{Column, QueryResults};

/// A client for Trino's statement-submission HTTP protocol. Holds no
/// session state beyond the coordinator URL and credentials — a fresh
/// `execute` call starts a fresh query.
pub struct TrinoClient {
    http: reqwest::Client,
    config: TrinoClientConfig,
}

impl TrinoClient {
    pub fn new(config: TrinoClientConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    /// Submit `sql` and drive `on_page` over every page in order, following
    /// `nextUri` to completion. Returns as soon as a page's `error` is set,
    /// or the last page (no `nextUri`) is reached. Shared by `execute`,
    /// `execute_schema` and `execute_json` so the paging loop has exactly
    /// one implementation.
    async fn follow_pages(
        &self,
        sql: &str,
        mut on_page: impl FnMut(&QueryResults) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let url = format!(
            "{}/v1/statement",
            self.config.base_url.trim_end_matches('/')
        );
        let mut page = self
            .send(self.http.post(&url).body(sql.to_string()))
            .await?;

        loop {
            if let Some(error) = &page.error {
                return Err(map_trino_error(error));
            }
            on_page(&page)?;
            match page.next_uri.clone() {
                Some(next_uri) => {
                    page = self.send(self.http.get(&next_uri)).await?;
                }
                None => break,
            }
        }
        Ok(())
    }

    /// Submit `sql`, follow `nextUri` to completion, and decode every page
    /// carrying `columns` + `data` into a `RecordBatch`.
    pub async fn execute(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let mut columns: Option<Vec<Column>> = None;
        let mut batches = Vec::new();

        self.follow_pages(sql, |page| {
            if let Some(cols) = &page.columns {
                columns = Some(cols.clone());
            }
            if let Some(data) = &page.data {
                let cols = columns.as_ref().ok_or_else(|| {
                    BackendError::execution_failed(
                        "trino",
                        "Trino returned result rows before any column metadata".to_string(),
                    )
                })?;
                if !data.is_empty() {
                    batches.push(rows_to_record_batch(cols, data)?);
                }
            }
            Ok(())
        })
        .await?;

        Ok(batches)
    }

    /// Submit `sql`, follow `nextUri` to completion so any error surfaces,
    /// and return the reported `(name, type)` pairs from the result schema.
    /// Decodes no rows — the schema-only audit leg needs types, not data,
    /// and an array cell's decode gap must never masquerade as a rejected
    /// probe.
    pub async fn execute_schema(&self, sql: &str) -> Result<Vec<(String, String)>, BackendError> {
        let mut columns: Option<Vec<Column>> = None;
        self.follow_pages(sql, |page| {
            if let Some(cols) = &page.columns {
                columns = Some(cols.clone());
            }
            Ok(())
        })
        .await?;

        let columns = columns.ok_or_else(|| {
            BackendError::execution_failed(
                "trino",
                format!("Trino returned no column metadata for: {sql}"),
            )
        })?;
        Ok(columns.into_iter().map(|c| (c.name, c.raw_type)).collect())
    }

    /// Submit `sql`, follow `nextUri` to completion, and return the reported
    /// `(name, raw_type)` columns together with the **undecoded** JSON rows.
    /// The value leg decodes each cell against its own declared type
    /// (`cell_from_trino_json` in `smelt-oracle-testkit`), deliberately not
    /// through `arrow_convert::trino_type_to_arrow`: that converter has no
    /// array/varbinary/interval arm, and a decode error there would
    /// masquerade as a rejected probe.
    pub async fn execute_json(
        &self,
        sql: &str,
    ) -> Result<(Vec<(String, String)>, Vec<Vec<serde_json::Value>>), BackendError> {
        let mut columns: Option<Vec<Column>> = None;
        let mut rows = Vec::new();
        self.follow_pages(sql, |page| {
            if let Some(cols) = &page.columns {
                columns = Some(cols.clone());
            }
            if let Some(data) = &page.data {
                rows.extend(data.iter().cloned());
            }
            Ok(())
        })
        .await?;

        let columns = columns.ok_or_else(|| {
            BackendError::execution_failed(
                "trino",
                format!("Trino returned no column metadata for: {sql}"),
            )
        })?;
        Ok((
            columns.into_iter().map(|c| (c.name, c.raw_type)).collect(),
            rows,
        ))
    }

    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<QueryResults, BackendError> {
        let mut builder = builder
            .header("X-Trino-User", &self.config.user)
            .header("X-Trino-Catalog", &self.config.catalog)
            .header("X-Trino-Schema", &self.config.schema)
            // Without this, the coordinator drops every parametric
            // date/time type to its unparameterized base type for legacy
            // client compatibility — a `timestamp(6)` column's *type* still
            // reports `timestamp`, but its *value* is also formatted at
            // millisecond precision (measured against a live tier: writing
            // `.123456` and reading back `.123` — not just a type-name
            // cosmetic). `PARAMETRIC_DATETIME` opts into full-precision
            // typing and formatting for the types `arrow_convert` decodes.
            .header("X-Trino-Client-Capabilities", "PARAMETRIC_DATETIME");
        if let Some(password) = &self.config.password {
            builder = builder.basic_auth(&self.config.user, Some(password));
        }

        let response = builder.send().await.map_err(|e| map_transport_error(&e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            return Err(BackendError::execution_failed(
                "trino",
                format!("HTTP {status} from Trino coordinator: {snippet}"),
            ));
        }

        response.json::<QueryResults>().await.map_err(|e| {
            BackendError::execution_failed("trino", format!("malformed Trino response: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_from_env() -> Option<TrinoClient> {
        let base_url = std::env::var("SMELT_TRINO_URL").ok()?;
        let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
        let catalog =
            std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
        let schema =
            std::env::var("SMELT_TRINO_SCHEMA").unwrap_or_else(|_| "smelt_dev".to_string());
        Some(TrinoClient::new(TrinoClientConfig {
            base_url,
            user,
            catalog,
            schema,
            password: None,
        }))
    }

    /// Live leg. Skips green when no Trino tier is exported. Proves
    /// `execute_json` pages to completion (like `execute`/`execute_schema`)
    /// while handing back undecoded JSON cells rather than Arrow-decoded
    /// ones — the shape the value-leg oracle needs for a type `arrow_convert`
    /// cannot decode (e.g. `array(...)`).
    #[tokio::test]
    async fn execute_json_returns_columns_and_raw_rows() {
        let Some(client) = client_from_env() else {
            eprintln!("SMELT_TRINO_URL unset — skipping execute_json_returns_columns_and_raw_rows");
            return;
        };
        let (columns, rows) = client
            .execute_json("SELECT CAST(1 AS BIGINT) AS a, CAST('x' AS VARCHAR) AS b")
            .await
            .expect("execute_json");
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].0, "a");
        assert_eq!(columns[1].0, "b");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[0][0], serde_json::json!(1));
        assert_eq!(rows[0][1], serde_json::json!("x"));
    }
}
