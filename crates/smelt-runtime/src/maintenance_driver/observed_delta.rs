use arrow::array::Array;
use smelt_backend::{Backend, BackendError};
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};
use smelt_state::ddl_duckdb;

/// Can `dialect` record an observed output delta at all?
///
/// **Derived from the availability layer, never hardcoded.** Every
/// `_smelt_observed_delta` write site asks this rather than comparing against
/// `SqlDialect::DuckDB` itself, so a dialect gaining the structure in
/// `realisable_state_structures` retires its guards in the same commit that
/// lands its emitters — the two can never drift apart again
/// (`docs/specs/state.md` §"The state-structure inventory";
/// `docs/outcomes/20260906-bigquery-correctness` decision log, 2026-09-10).
///
/// Where this is `false`, a write site **skips the record and proceeds with
/// the write**: it must not refuse. The read side already treats an absent
/// delta as a legal widen-never-narrow fallback trigger
/// ([`read_observed_delta`]), so an unrecorded window costs downstream
/// precision — a wider recompute — and never correctness. Refusing instead
/// was the 2026-09-10 hard stop that halted `examples/github_activity` on
/// BigQuery at `silver.events_deduped`.
pub fn records_observed_deltas(dialect: SqlDialect) -> bool {
    realisable_state_structures(dialect).contains(&StateStructure::ObservedOutputDeltas)
}

/// Read the exact observed-delta changed-key set an upstream driving model
/// edge recorded for `[window_start, window_end)` (T5, Group D). `None` = no
/// row was ever recorded for this window — the "pre-D2 upstream" / never-
/// recorded case, the trigger for the widen-never-narrow fallback — distinct
/// from `Some(&[])`'s "recorded and present-and-empty" (a fully-suppressed
/// upstream run; `incremental_models.md` §"The graph layer" — "Empty and
/// absent are distinct").
///
/// Gated on [`records_observed_deltas`], like every other
/// `_smelt_observed_delta` consumer. A missing delta is always a legal
/// widen-never-narrow fallback trigger, so a dialect that cannot record one
/// reads back `None` rather than erroring — the same posture the write side
/// now takes (it skips the record; before 2026-09-10 it refused the run).
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

/// Decode a single string-array column of an observed-delta result batch into
/// owned strings.
///
/// **Fail-loud on an unrecognised shape, never silently empty.** The previous
/// form early-returned an empty vector when the column was missing or was not
/// a `ListArray` of `StringArray`, which was harmless while DuckDB was the
/// only producer and is a silent-narrowing hazard now that it is not: a
/// downstream consumer cannot tell an empty decode from a genuinely empty
/// delta, so a shape this function does not understand would restrict a
/// recompute to *no* keys instead of widening (`CLAUDE.md` §"Fail-loud
/// discipline").
///
/// Both arrow list widths and both string widths are accepted, because the
/// producer is now an adapter rather than an in-process engine: BigQuery's
/// results arrive through `pyarrow` (`result.to_arrow()` → `to_batches()` →
/// `RecordBatch::from_pyarrow_bound`), where a `REPEATED STRING` field is
/// conventionally `list<item: string>` but a large-offset variant is a
/// representation detail no caller should depend on. A NULL list entry decodes
/// as empty: BigQuery cannot store a NULL array at all (a NULL written to an
/// `ARRAY` column reads back empty), and empty-vs-absent is carried by row
/// presence, not by a column value.
fn decode_string_list_column(
    batch: &arrow::array::RecordBatch,
    column: &str,
) -> std::result::Result<Vec<String>, BackendError> {
    use arrow::array::{LargeListArray, LargeStringArray, ListArray, StringArray};

    let col = batch.column_by_name(column).ok_or_else(|| {
        BackendError::execution_failed(
            "observed-delta decode",
            format!(
                "the observed-delta row carries no '{column}' column (columns: {:?}) — \
                 refusing rather than decoding an empty delta, which a consumer cannot \
                 distinguish from a genuinely empty one",
                batch
                    .schema()
                    .fields()
                    .iter()
                    .map(|f| f.name())
                    .collect::<Vec<_>>()
            ),
        )
    })?;

    let mut out = Vec::new();
    let mut push_values =
        |values: arrow::array::ArrayRef| -> std::result::Result<(), BackendError> {
            if let Some(strings) = values.as_any().downcast_ref::<StringArray>() {
                for j in 0..strings.len() {
                    if !strings.is_null(j) {
                        out.push(strings.value(j).to_string());
                    }
                }
                Ok(())
            } else if let Some(strings) = values.as_any().downcast_ref::<LargeStringArray>() {
                for j in 0..strings.len() {
                    if !strings.is_null(j) {
                        out.push(strings.value(j).to_string());
                    }
                }
                Ok(())
            } else {
                Err(BackendError::execution_failed(
                    "observed-delta decode",
                    format!(
                        "observed-delta column '{column}' holds a list of {:?}, not of strings",
                        values.data_type()
                    ),
                ))
            }
        };

    if let Some(list) = col.as_any().downcast_ref::<ListArray>() {
        for i in 0..list.len() {
            if !list.is_null(i) {
                push_values(list.value(i))?;
            }
        }
    } else if let Some(list) = col.as_any().downcast_ref::<LargeListArray>() {
        for i in 0..list.len() {
            if !list.is_null(i) {
                push_values(list.value(i))?;
            }
        }
    } else {
        return Err(BackendError::execution_failed(
            "observed-delta decode",
            format!(
                "observed-delta column '{column}' has arrow type {:?}, which is not a list — \
                 refusing rather than decoding an empty delta",
                col.data_type()
            ),
        ));
    }
    Ok(out)
}

/// Read the exact observed delta (both `changed_keys` and `partitions`) an
/// upstream driving model edge recorded for `[window_start, window_end)` —
/// the single decode site [`read_observed_delta_changed_keys`] and
/// [`crate::propagation::load_observed_delta_lookup`] both re-express
/// themselves over. `None` = no row was ever recorded for this window (the
/// widen-never-narrow fallback trigger); `Some` — even with both vectors
/// empty — means a row exists (§"Empty and absent are distinct").
///
/// Gated on [`records_observed_deltas`], matching every other
/// `_smelt_observed_delta` consumer: a missing delta on the read side is
/// always a legal fallback trigger, so a dialect that cannot record one
/// reads back `None` rather than erroring.
pub async fn read_observed_delta(
    backend: &dyn Backend,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> std::result::Result<Option<ddl_duckdb::ObservedDelta>, BackendError> {
    if !records_observed_deltas(backend.dialect()) {
        return Ok(None);
    }
    let dialect = backend.dialect();
    let ensure_sql = smelt_state::observed_delta::observed_delta_table_ddl(dialect, schema)
        .map_err(|e| BackendError::unsupported(dialect.name(), e.to_string()))?;
    backend.execute_sql(&ensure_sql).await?;

    let select_sql = smelt_state::observed_delta::observed_delta_select_sql(
        dialect,
        schema,
        model,
        window_start,
        window_end,
    )
    .map_err(|e| BackendError::unsupported(dialect.name(), e.to_string()))?;
    let batches = backend.execute_sql(&select_sql).await?;
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    if total_rows == 0 {
        return Ok(None);
    }

    let mut changed_keys = Vec::new();
    let mut partitions = Vec::new();
    for batch in &batches {
        changed_keys.extend(decode_string_list_column(batch, "changed_keys")?);
        partitions.extend(decode_string_list_column(batch, "partitions")?);
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

#[cfg(test)]
mod decode_tests {
    use super::decode_string_list_column;
    use arrow::array::{
        ArrayRef, Int64Array, LargeListArray, LargeStringArray, ListArray, RecordBatch, StringArray,
    };
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    fn batch(column: &str, array: ArrayRef) -> RecordBatch {
        let schema = Schema::new(vec![Field::new(column, array.data_type().clone(), true)]);
        RecordBatch::try_new(Arc::new(schema), vec![array]).expect("batch")
    }

    /// The shape BigQuery's adapter is expected to produce for a
    /// `REPEATED STRING` field: `list<…: string>`. The child field is
    /// deliberately *not* named `item` here — a producer's field naming must
    /// not change what decodes.
    #[test]
    fn a_plain_list_of_strings_decodes() {
        let values = StringArray::from(vec!["a", "b"]);
        let field = Arc::new(Field::new("element", DataType::Utf8, true));
        let offsets = arrow::buffer::OffsetBuffer::new(vec![0, 2].into());
        let array: ArrayRef = Arc::new(ListArray::new(
            field,
            offsets,
            Arc::new(values) as ArrayRef,
            None,
        ));
        let decoded =
            decode_string_list_column(&batch("changed_keys", array), "changed_keys").expect("ok");
        assert_eq!(decoded, vec!["a".to_string(), "b".to_string()]);
    }

    /// A large-offset representation is a producer detail, not a semantic
    /// difference — it must decode identically rather than silently empty.
    #[test]
    fn a_large_list_of_large_strings_decodes_identically() {
        let values = LargeStringArray::from(vec!["a", "b"]);
        let field = Arc::new(Field::new("element", DataType::LargeUtf8, true));
        let offsets = arrow::buffer::OffsetBuffer::new(vec![0i64, 2].into());
        let array: ArrayRef = Arc::new(LargeListArray::new(
            field,
            offsets,
            Arc::new(values) as ArrayRef,
            None,
        ));
        let decoded =
            decode_string_list_column(&batch("changed_keys", array), "changed_keys").expect("ok");
        assert_eq!(decoded, vec!["a".to_string(), "b".to_string()]);
    }

    /// The silent-empty failure mode this outcome exists to catch: a shape the
    /// decoder does not understand must refuse, not report "no keys changed".
    #[test]
    fn an_unrecognised_column_shape_refuses_instead_of_decoding_empty() {
        let array: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
        let err = decode_string_list_column(&batch("changed_keys", array), "changed_keys")
            .expect_err("a non-list column must refuse");
        assert!(err.to_string().contains("not a list"), "{err}");

        let values = Int64Array::from(vec![1, 2]);
        let field = Arc::new(Field::new("element", DataType::Int64, true));
        let offsets = arrow::buffer::OffsetBuffer::new(vec![0, 2].into());
        let array: ArrayRef = Arc::new(ListArray::new(
            field,
            offsets,
            Arc::new(values) as ArrayRef,
            None,
        ));
        let err = decode_string_list_column(&batch("changed_keys", array), "changed_keys")
            .expect_err("a list of non-strings must refuse");
        assert!(err.to_string().contains("not of strings"), "{err}");

        let array: ArrayRef = Arc::new(StringArray::from(vec!["x"]));
        let err = decode_string_list_column(&batch("other", array), "changed_keys")
            .expect_err("a missing column must refuse");
        assert!(
            err.to_string().contains("no 'changed_keys' column"),
            "{err}"
        );
    }

    /// A NULL list entry decodes as empty rather than refusing: BigQuery
    /// cannot store a NULL array, and empty-vs-absent is row presence.
    #[test]
    fn a_null_list_entry_decodes_as_empty() {
        let values = StringArray::from(vec!["a"]);
        let field = Arc::new(Field::new("element", DataType::Utf8, true));
        let offsets = arrow::buffer::OffsetBuffer::new(vec![0, 1, 1].into());
        let nulls = arrow::buffer::NullBuffer::from(vec![true, false]);
        let array: ArrayRef = Arc::new(ListArray::new(
            field,
            offsets,
            Arc::new(values) as ArrayRef,
            Some(nulls),
        ));
        let decoded =
            decode_string_list_column(&batch("partitions", array), "partitions").expect("ok");
        assert_eq!(decoded, vec!["a".to_string()]);
    }
}
