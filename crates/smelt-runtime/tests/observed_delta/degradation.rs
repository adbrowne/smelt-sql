//! Degradation leg of T5: what a dialect that cannot realise
//! `StateStructure::ObservedOutputDeltas` does with a change-suppressed
//! keyed fold.
//!
//! Split out of `main.rs` so the recording tests (which need a real DuckDB)
//! and the degradation test (which needs a fake non-DuckDB backend and its
//! ~110 lines of trait stubs) do not share one oversized file. Helpers come
//! from the parent target.

use super::{key_suppression, max_score_rule, no_retry_policy, one_step};
use smelt_backend::{Backend, PartitionRange};
use smelt_runtime::maintenance_driver::run_windowed_keyed_maintenance;
use smelt_runtime::probes::ProbePolicy;

#[derive(Default)]
struct KeyedNonDuckDbBackend {
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Backend for KeyedNonDuckDbBackend {
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        self.calls.lock().unwrap().push(sql.to_string());
        Ok(vec![])
    }
    async fn create_table_as(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn create_view_as(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn drop_table_if_exists(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn drop_view_if_exists(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn get_row_count(&self, _: &str, _: &str) -> Result<usize, smelt_backend::BackendError> {
        // Reached only because the run now COMPLETES. Before the 2026-09-10
        // fix the driver refused before any write, so this was `unimplemented!()`.
        Ok(0)
    }
    async fn get_preview(
        &self,
        _: &str,
        _: &str,
        _: usize,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn table_exists(&self, _: &str, _: &str) -> Result<bool, smelt_backend::BackendError> {
        // The target already exists — so the driver reaches the merge
        // (not the first-run `CREATE TABLE ... AS`) branch, where the
        // dialect refusal lives.
        Ok(true)
    }
    async fn ensure_schema(&self, _: &str) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    fn dialect(&self) -> smelt_backend::SqlDialect {
        smelt_backend::SqlDialect::SparkSQL
    }
    fn capabilities(&self) -> smelt_backend::BackendCapabilities {
        // The write-mechanism resolution (`resolve_keyed_write_mechanism`,
        // 27g) now consults capabilities before any backend call — this
        // fake backend's own SparkSQL dialect answers truthfully (Spark can
        // run MERGE) so the driver reaches this test's actual target: the
        // non-DuckDB dialect refusal inside the observed-delta branch, not
        // an unrelated panic here.
        smelt_backend::BackendCapabilities::spark()
    }
    async fn load_table(
        &self,
        _: &str,
        _: &str,
        _: arrow::datatypes::SchemaRef,
        _: Vec<arrow::array::RecordBatch>,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn delete_partitions(
        &self,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn insert_into_from_query(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn insert_overwrite(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
}

/// A change-suppressed keyed fold on a dialect that cannot realise
/// `StateStructure::ObservedOutputDeltas` **skips the record and still
/// writes** — it does not refuse the run.
///
/// This test asserted the opposite until 2026-09-10, when the refusal turned
/// out to be the hard stop that halted `examples/github_activity` on BigQuery
/// at `silver.events_deduped`
/// (`docs/outcomes/20260906-bigquery-correctness` decision log). Skipping is
/// sound because an absent delta is already defined as a legal
/// widen-never-narrow fallback trigger on the read side
/// (`read_observed_delta` returns `None` here), so the cost is downstream
/// precision, never correctness. Non-vacuity for the skip is the whole rest
/// of this file: every test above records the delta on DuckDB, where the
/// structure IS realisable.
#[tokio::test]
async fn keyed_fold_suppressed_recording_degrades_on_a_non_duckdb_backend() {
    let backend = KeyedNonDuckDbBackend::default();
    let suppression = key_suppression(&["score"]);
    let steps = one_step("2026-01-01", "2026-01-02");

    run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &max_score_rule(),
        None,
        &suppression,
        None,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("a dialect without the observed-delta structure must degrade, not refuse");

    let calls = backend.calls.lock().unwrap();
    assert!(
        !calls.iter().any(|c| c.contains("_smelt_observed_delta")),
        "no observed-delta SQL may reach a backend that cannot realise the structure: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("MERGE")),
        "the merge itself must still run: {calls:?}"
    );
}
