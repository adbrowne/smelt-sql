//! Shared live-tier harness for `smelt-backend-trino` integration tests
//! (`capability_probes.rs`, `merge_clause_forms.rs`): connect to
//! `SMELT_TRINO_URL`, create a unique per-run schema, and tear it down.
//! Gated on `SMELT_TRINO_URL`: unset, [`live_env_or_skip`] returns `None`
//! and the caller should skip green. Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test <name>
//!   bash scripts/trino-down.sh
//!
//! Not every helper here is used by every test binary that includes this
//! module (`mod common;` compiles a fresh copy per binary) — allowed rather
//! than split further, since the whole point is one shared harness.
#![allow(dead_code)]

use arrow::array::{Array, Int64Array, StringArray};
use smelt_backend::Backend;
use smelt_backend_trino::{TrinoBackend, TrinoClientConfig};

pub struct LiveEnv {
    pub backend: TrinoBackend,
    pub catalog: String,
    pub schema: String,
}

fn unique_schema(suffix: &str) -> String {
    let base = std::env::var("SMELT_TRINO_SCHEMA").unwrap_or_else(|_| "smelt_dev".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{base}_{suffix}_{}_{nanos}", std::process::id())
}

/// Connect and create the run's isolated schema, or `None` when
/// `SMELT_TRINO_URL` is unset (the caller should skip green). `suffix`
/// distinguishes one test file's schemas from another's under concurrent
/// `cargo test` runs.
pub async fn live_env_or_skip(test_name: &str, suffix: &str) -> Option<LiveEnv> {
    let Ok(base_url) = std::env::var("SMELT_TRINO_URL") else {
        eprintln!("Skipping {test_name} — set SMELT_TRINO_URL");
        return None;
    };
    let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
    let catalog = std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
    let schema = unique_schema(suffix);

    let backend = TrinoBackend::new(TrinoClientConfig {
        base_url,
        user,
        catalog: catalog.clone(),
        schema: schema.clone(),
        password: None,
    });
    backend
        .ensure_schema(&schema)
        .await
        .unwrap_or_else(|e| panic!("ensure_schema must succeed against a live tier: {e}"));

    Some(LiveEnv {
        backend,
        catalog,
        schema,
    })
}

pub async fn drop_schema(env: &LiveEnv) {
    let _ = env
        .backend
        .execute_sql(&format!(
            "DROP SCHEMA IF EXISTS \"{}\".\"{}\" CASCADE",
            env.catalog, env.schema
        ))
        .await;
}

impl LiveEnv {
    pub fn q(&self, name: &str) -> String {
        format!("\"{}\".\"{}\".\"{name}\"", self.catalog, self.schema)
    }

    pub async fn ok(&self, sql: &str) -> bool {
        self.backend.execute_sql(sql).await.is_ok()
    }

    /// The verbatim coordinator error text for a statement expected to fail,
    /// or `None` if it unexpectedly succeeded.
    pub async fn err_text(&self, sql: &str) -> Option<String> {
        self.backend
            .execute_sql(sql)
            .await
            .err()
            .map(|e| e.to_string())
    }

    /// The first `i64` column of every result row, in the order the
    /// coordinator returned them.
    pub async fn select_i64_col(&self, sql: &str) -> Vec<i64> {
        let batches = self
            .backend
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("query must succeed: {sql}: {e}"));
        let mut out = Vec::new();
        for batch in &batches {
            let col = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap_or_else(|| panic!("first column is not Int64: {sql}"));
            for i in 0..col.len() {
                out.push(col.value(i));
            }
        }
        out
    }

    /// The first `varchar` column of every result row, in the order the
    /// coordinator returned them.
    pub async fn select_string_col(&self, sql: &str) -> Vec<String> {
        let batches = self
            .backend
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("query must succeed: {sql}: {e}"));
        let mut out = Vec::new();
        for batch in &batches {
            let col = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap_or_else(|| panic!("first column is not Utf8: {sql}"));
            for i in 0..col.len() {
                out.push(col.value(i).to_string());
            }
        }
        out
    }
}
