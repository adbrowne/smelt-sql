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

/// A non-DuckDB backend that records every statement it is handed. The
/// dialect is a field rather than a constant because the two legs this file
/// now holds differ only in it: Spark cannot realise the observed-delta
/// record and degrades, BigQuery realises it and records.
struct KeyedNonDuckDbBackend {
    calls: std::sync::Mutex<Vec<String>>,
    dialect: smelt_backend::SqlDialect,
}

impl KeyedNonDuckDbBackend {
    fn new(dialect: smelt_backend::SqlDialect) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            dialect,
        }
    }
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
        self.dialect
    }
    fn capabilities(&self) -> smelt_backend::BackendCapabilities {
        // The write-mechanism resolution (`resolve_keyed_write_mechanism`,
        // 27g) now consults capabilities before any backend call — this
        // fake backend's own SparkSQL dialect answers truthfully (Spark can
        // run MERGE) so the driver reaches this test's actual target: the
        // non-DuckDB dialect refusal inside the observed-delta branch, not
        // an unrelated panic here.
        match self.dialect {
            smelt_backend::SqlDialect::BigQuery => smelt_backend::BackendCapabilities::bigquery(),
            _ => smelt_backend::BackendCapabilities::spark(),
        }
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
    let backend = KeyedNonDuckDbBackend::new(smelt_backend::SqlDialect::SparkSQL);
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

/// The same change-suppressed keyed fold on **BigQuery** no longer degrades:
/// the structure is realisable there, so the record is emitted — in GoogleSQL,
/// not DuckDB SQL.
///
/// This is the other side of the test above, and the pair is what keeps each
/// non-vacuous: one dialect skips because it cannot realise the structure, the
/// other records because it can.
#[tokio::test]
async fn keyed_fold_suppressed_recording_is_realised_on_bigquery() {
    let backend = KeyedNonDuckDbBackend::new(smelt_backend::SqlDialect::BigQuery);
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
    .expect("BigQuery realises the observed-delta record");

    let calls = backend.calls.lock().unwrap();
    let record = calls
        .iter()
        .find(|c| c.contains("_smelt_observed_delta") && c.starts_with("MERGE"))
        .unwrap_or_else(|| panic!("the observed-delta record must be emitted: {calls:?}"));
    // GoogleSQL, not DuckDB SQL — the whole point of the dispatch.
    assert!(record.contains("IGNORE NULLS"), "{record}");
    assert!(!record.contains("ON CONFLICT"), "{record}");
    assert!(!record.contains("FILTER (WHERE"), "{record}");
    assert!(!record.contains("::VARCHAR[]"), "{record}");
    assert!(
        calls
            .iter()
            .any(|c| c.contains("CREATE TABLE IF NOT EXISTS `main._smelt_observed_delta`")),
        "the backticked GoogleSQL ensure-DDL must be emitted: {calls:?}"
    );
}

/// The run-layer predicate is derived from the availability layer, so the row
/// flip *is* the switch. Exhaustive over the dialects so a new one has to be
/// considered here rather than defaulting.
#[test]
fn records_observed_deltas_follows_the_availability_row() {
    use smelt_backend::SqlDialect;
    use smelt_runtime::maintenance_driver::records_observed_deltas;

    for dialect in [
        SqlDialect::DuckDB,
        SqlDialect::BigQuery,
        SqlDialect::SparkSQL,
    ] {
        let expected = match dialect {
            SqlDialect::DuckDB | SqlDialect::BigQuery => true,
            // Permanent, not pending: Delta has no cross-table transaction, so
            // the record and its write cannot commit together.
            SqlDialect::SparkSQL => false,
        };
        assert_eq!(
            records_observed_deltas(dialect),
            expected,
            "{dialect:?} disagrees with docs/specs/state.md §\"Which dialects realise which \
             structure\""
        );
    }
}
