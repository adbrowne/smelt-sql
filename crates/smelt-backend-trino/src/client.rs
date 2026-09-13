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

    /// Submit `sql`, follow `nextUri` to completion, and decode every page
    /// carrying `columns` + `data` into a `RecordBatch`. Returns as soon as
    /// a page's `error` is set, or the last page (no `nextUri`) is reached.
    pub async fn execute(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let url = format!(
            "{}/v1/statement",
            self.config.base_url.trim_end_matches('/')
        );
        let mut page = self
            .send(self.http.post(&url).body(sql.to_string()))
            .await?;

        let mut columns: Option<Vec<Column>> = None;
        let mut batches = Vec::new();

        loop {
            if let Some(error) = &page.error {
                return Err(map_trino_error(error));
            }
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

            match page.next_uri.clone() {
                Some(next_uri) => {
                    page = self.send(self.http.get(&next_uri)).await?;
                }
                None => break,
            }
        }

        Ok(batches)
    }

    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<QueryResults, BackendError> {
        let mut builder = builder
            .header("X-Trino-User", &self.config.user)
            .header("X-Trino-Catalog", &self.config.catalog)
            .header("X-Trino-Schema", &self.config.schema);
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
