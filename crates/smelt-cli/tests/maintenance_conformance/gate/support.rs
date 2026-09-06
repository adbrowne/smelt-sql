//! Shared plumbing for the conformance gate's submodules: the no-retry reporter/policy every direct maintenance-driver call site uses, and the table snapshot helper.

use smelt_backend::Backend;

/// A retry policy that never retries — this conformance gate drives a real
/// DuckDB backend directly rather than going through `execute_project`, so
/// there is no `ExecuteRequest`/run reporter to derive one from
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). `retry_max: 0`
/// keeps every call site's behaviour identical to before retry coverage was
/// extended to these maintenance-driver entry points.
pub(crate) const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
pub(crate) fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "maintenance-conformance-gate",
        model_name: "maintenance-conformance-gate",
        reporter: &NO_OP_REPORTER,
    }
}

/// Snapshot `main.<table>`'s full contents as sorted, comparable text rows —
/// the zero-write redelivery step's before/after equality check.
pub(crate) async fn snapshot_table_rows(
    backend: &dyn Backend,
    table: &str,
) -> anyhow::Result<Vec<Vec<String>>> {
    let batches = backend
        .execute_sql(&format!("SELECT * FROM main.{table} ORDER BY ALL"))
        .await?;
    let mut rows = Vec::new();
    for batch in &batches {
        for row_idx in 0..batch.num_rows() {
            let mut row = Vec::new();
            for col in batch.columns() {
                row.push(arrow::util::display::array_value_to_string(col, row_idx)?);
            }
            rows.push(row);
        }
    }
    Ok(rows)
}
