use arrow::array::Array;
use smelt_backend::{Backend, BackendError};
use smelt_dialect::SqlDialect;
use smelt_state::ddl_duckdb;

/// Read the exact observed-delta changed-key set an upstream driving model
/// edge recorded for `[window_start, window_end)` (T5, Group D). `None` = no
/// row was ever recorded for this window — the "pre-D2 upstream" / never-
/// recorded case, the trigger for the widen-never-narrow fallback — distinct
/// from `Some(&[])`'s "recorded and present-and-empty" (a fully-suppressed
/// upstream run; `incremental_models.md` §"The graph layer" — "Empty and
/// absent are distinct").
///
/// DuckDB-only, matching every other `_smelt_observed_delta` consumer in
/// this module (`execute_column_scoped_write_with_observed_delta` above).
/// Unlike that function's *write*-side capability gap (a hard error — the
/// caller asked for a technique the backend cannot provide), a missing
/// delta on the *read* side is always a legal fallback trigger, so a non-
/// DuckDB backend reads back `None` rather than erroring.
pub async fn read_observed_delta_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> std::result::Result<Option<Vec<String>>, BackendError> {
    Ok(
        read_observed_delta(backend, schema, model, window_start, window_end)
            .await?
            .map(|od| od.changed_keys),
    )
}

/// Decode a single `VARCHAR[]` column of an observed-delta result batch
/// into owned strings, skipping a null list entry or a non-string-array
/// column shape (defensive — the DDL guarantees `VARCHAR[] NOT NULL`, but
/// this never panics on an unexpected shape).
fn decode_string_list_column(batch: &arrow::array::RecordBatch, column: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(col) = batch.column_by_name(column) else {
        return out;
    };
    let Some(list) = col.as_any().downcast_ref::<arrow::array::ListArray>() else {
        return out;
    };
    for i in 0..list.len() {
        if list.is_null(i) {
            continue;
        }
        let values = list.value(i);
        let Some(strings) = values.as_any().downcast_ref::<arrow::array::StringArray>() else {
            continue;
        };
        for j in 0..strings.len() {
            if !strings.is_null(j) {
                out.push(strings.value(j).to_string());
            }
        }
    }
    out
}

/// Read the exact observed delta (both `changed_keys` and `partitions`) an
/// upstream driving model edge recorded for `[window_start, window_end)` —
/// the single decode site [`read_observed_delta_changed_keys`] and
/// [`crate::propagation::load_observed_delta_lookup`] both re-express
/// themselves over. `None` = no row was ever recorded for this window (the
/// widen-never-narrow fallback trigger); `Some` — even with both vectors
/// empty — means a row exists (§"Empty and absent are distinct").
///
/// DuckDB-only, matching every other `_smelt_observed_delta` consumer in
/// this module: a missing delta on the read side is always a legal
/// fallback trigger, so a non-DuckDB backend reads back `None` rather than
/// erroring.
pub async fn read_observed_delta(
    backend: &dyn Backend,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> std::result::Result<Option<ddl_duckdb::ObservedDelta>, BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Ok(None);
    }
    let ensure_sql = ddl_duckdb::generate_observed_delta_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let select_sql =
        ddl_duckdb::generate_observed_delta_select_sql(schema, model, window_start, window_end);
    let batches = backend.execute_sql(&select_sql).await?;
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    if total_rows == 0 {
        return Ok(None);
    }

    let mut changed_keys = Vec::new();
    let mut partitions = Vec::new();
    for batch in &batches {
        changed_keys.extend(decode_string_list_column(batch, "changed_keys"));
        partitions.extend(decode_string_list_column(batch, "partitions"));
    }
    Ok(Some(ddl_duckdb::ObservedDelta {
        changed_keys,
        partitions,
    }))
}

// ── F3: fingerprint sidecar — synthesized external change feed ─────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F3;
// `docs/specs/sources.md` §"The fingerprint sidecar")
//
// Builds and consumes the row-content fingerprint sidecar for a
// `mutable_snapshot` external source with no native change feed: the diff
// (`diff_fingerprint_sidecar_changed_keys`) synthesizes an exact changed-key
// set from a full re-scan of the source compared against the sidecar's
// stored digests; the refresh (`refresh_fingerprint_sidecar`) then brings
// the sidecar's stored digests up to date with the source's current
// content, riding in the same backend transaction as the write that
// consumed the diff. Wiring this changed-key set into the maintenance
// plan's own trigger/technique selection (deciding WHEN a live run uses the
// sidecar-derived delta instead of the whole-table one) is a licence change
// scoped to a later phase (T3 over external sources) — these functions are
// a standalone, independently-tested capability today, matching P4's own
// "no consumer reads it yet" framing (`model_properties.md` §"Fingerprint
// projection").
