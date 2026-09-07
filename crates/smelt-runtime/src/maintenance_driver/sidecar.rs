use arrow::array::Array;
use smelt_backend::{maintenance_dialect, Backend, BackendError};
use smelt_logical::analysis::fingerprint;
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::maintenance::emit::{
    emit_fingerprint_digest_select, emit_fingerprint_sidecar_diff, emit_repair_group_digest_select,
    emit_repair_group_sidecar_diff, MaintenanceDialect, StatementGroup,
};
use smelt_state::ddl_duckdb;

/// Resolve which columns a fingerprint sidecar digests for one `(model,
/// external source)` pair: the P4 verdict's own column set, or — fail-
/// closed — `all_source_columns` when the verdict is `FullRow`
/// (`model_properties.md` §"Fingerprint projection": "an unprojectable
/// consumption ... yields `FullRow`, never a guessed subset"). Pure data —
/// no sidecar/digest machinery, matching
/// `smelt_logical::maintenance::derive`'s own "pure data, no
/// sidecar/digest machinery here" framing for the P4 derivation itself.
pub fn resolve_fingerprint_digest_columns(
    projection: &FingerprintProjection,
    all_source_columns: &[String],
) -> Vec<String> {
    match projection {
        FingerprintProjection::Columns(cols) => cols.iter().cloned().collect(),
        FingerprintProjection::FullRow { .. } => all_source_columns.to_vec(),
    }
}

// ── F4: fingerprint sidecar invalidation ────────────────────────────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F4
// — "Sidecar invalidation"; `docs/specs/sources.md` §"The fingerprint
// sidecar")
//
// A sidecar partition's stored digests are only trustworthy comparanda for
// a diff if nothing that could change what "the same row" or "the same
// digest" means has changed underneath it since the last refresh. Three
// independent things can invalidate that trust, any one of which must widen
// the next diff to "everything in the source is changed" — never a
// narrower, partially-trusted comparison, and never a silent skip:
//
// - the digest-construction version (`FINGERPRINT_SIDECAR_DIGEST_VERSION`)
//   — bumped only when `emit_fingerprint_digest_select`'s own hashing
//   scheme changes shape;
// - the P4 fingerprint projection's identity (already the sidecar's own
//   partition key — a projection change lands in a fresh, unpopulated
//   partition by construction, no extra mechanism needed);
// - the consuming model's own SQL definition, hashed the same way
//   `IntervalStore::get_or_create` invalidates covered intervals on a
//   model edit (`smelt_state::intervals::compute_model_hash`) — this is
//   the one trigger that can go stale WITHOUT a fresh partition, since two
//   different model SQL texts can resolve the identical P4 projection.

/// Digest-construction version for the fingerprint sidecar's stored digests
/// (`emit_fingerprint_digest_select`'s `sha256(...)` shape). Part of every
/// partition's identity stamp — bump this only when that construction
/// changes in a way that makes a previously stored digest no longer
/// comparable to a freshly computed one, so every stamp stored under the
/// old scheme is detected as mismatched (never silently trusted) on the
/// next diff.
const FINGERPRINT_SIDECAR_DIGEST_VERSION: &str = "v1";

/// Compute the fingerprint sidecar's identity stamp for one `(model,
/// external source)` pair — the combined value invalidation compares the
/// freshly computed one against on every run, mirroring
/// `IntervalStore::get_or_create`'s `model_hash != model_hash`
/// invalidation-on-mismatch precedent (`smelt_state::intervals`). Combines:
/// - [`FINGERPRINT_SIDECAR_DIGEST_VERSION`] (the digest-algorithm version);
/// - `projection_identity` (the caller's already-resolved P4 projection
///   identity — `smelt_logical::analysis::fingerprint::projection_identity`);
/// - a hash of `model_sql` (the consuming model's own SQL text), via
///   [`smelt_state::intervals::compute_model_hash`] — the same hash
///   `IntervalStore` uses to invalidate covered intervals on a model edit.
///
/// Any one of the three inputs changing produces a different stamp — this
/// is deliberately coarse (a model edit unrelated to this source still
/// invalidates the sidecar for it) rather than attempting to prove the edit
/// was irrelevant: the fail-loud/widen-never-narrow posture this codebase
/// takes everywhere else in the maintenance layer.
pub fn compute_fingerprint_sidecar_stamp(projection_identity: &str, model_sql: &str) -> String {
    let model_hash = smelt_state::intervals::compute_model_hash(model_sql);
    format!("{FINGERPRINT_SIDECAR_DIGEST_VERSION}:{projection_identity}:{model_hash}")
}

/// Read-side: the synthesized changed-key set the fingerprint sidecar
/// derives for `(source_address, source_table)` under `projection` (the
/// caller's already-resolved P4 fingerprint projection;
/// `all_source_columns` resolves the fail-closed `FullRow` case). Ensures
/// the sidecar table exists, then runs the emitter-authored diff query
/// (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`).
///
/// An absent sidecar makes every current source row "changed" by
/// construction (`docs/specs/sources.md` §"The fingerprint sidecar" —
/// "First run and `--full-refresh`") — no special-casing needed here, the
/// diff query's own `FULL OUTER JOIN` produces that result against an
/// empty (or not-yet-created) sidecar partition.
///
/// Gated on a target declaring `supports_fingerprint_sidecar`, matching
/// every other `_smelt_fingerprint_sidecar`/`_smelt_observed_delta` consumer
/// in this module. Unlike `read_observed_delta_changed_keys`'s read-side
/// fallback (a missing delta is always a legal widen-never-narrow trigger,
/// so it reads back `None` for a target lacking the capability), a caller
/// asking for a sidecar diff at all has already chosen the sidecar-backed
/// path — a target without the capability here fails loudly
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "DuckDB-scoped
/// today ... a target lacking the capability fails loud rather than
/// silently skipping the sidecar"). The DDL owner
/// (`ddl_duckdb::generate_fingerprint_sidecar_table_ddl`) is still
/// DuckDB-shaped, so a second backend declaring the capability needs its
/// own DDL first.
///
/// `model_sql` is the consuming model's own SQL text — folded into the
/// partition's identity stamp ([`compute_fingerprint_sidecar_stamp`]) so a
/// model-definition edit invalidates this partition even when it leaves
/// the P4 projection's column set (and therefore `identity`) unchanged.
/// Before running the diff, this checks whether any stored row's stamp no
/// longer matches the freshly computed one and, if so, logs a `tracing::
/// warn!` — the diff itself always structurally excludes a mismatched row
/// from the comparison (`emit_fingerprint_sidecar_diff`'s own `stamp =
/// '...'` filter), so this check changes no behaviour, it only makes an
/// invalidation loud rather than silent.
#[allow(clippy::too_many_arguments)]
pub async fn diff_fingerprint_sidecar_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    source_key: &[String],
    projection: &FingerprintProjection,
    all_source_columns: &[String],
    model_sql: &str,
    consumer_address: &str,
) -> std::result::Result<Vec<String>, BackendError> {
    if !backend.capabilities().supports_fingerprint_sidecar {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "fingerprint-sidecar diff for a mutable_snapshot external source (F3)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let digest_columns = resolve_fingerprint_digest_columns(projection, all_source_columns);
    let identity = fingerprint::projection_identity(projection);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);

    let stale_check_sql = ddl_duckdb::generate_fingerprint_sidecar_stale_check_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &stamp,
    );
    let stale_rows = backend.execute_sql(&stale_check_sql).await?;
    if stale_rows.iter().any(|batch| batch.num_rows() > 0) {
        tracing::warn!(
            source_address,
            projection_identity = %identity,
            consumer_address,
            "fingerprint sidecar stamp mismatch detected (model definition, P4 projection, or \
             digest version changed — or the stored stamp was corrupted); treating the stale \
             partition as absent and rebuilding via the whole-table delta"
        );
    }

    let sidecar_table = format!("{schema}.{}", ddl_duckdb::FINGERPRINT_SIDECAR_TABLE_NAME);
    let dialect = maintenance_dialect(backend.dialect());
    let diff_sql = emit_fingerprint_sidecar_diff(
        source_table,
        source_key,
        &digest_columns,
        &sidecar_table,
        source_address,
        &identity,
        consumer_address,
        &stamp,
        dialect,
    );
    let batches = backend.execute_sql(&diff_sql).await?;
    Ok(extract_delta_keys(&batches))
}

/// Extract every non-NULL `delta_key` value from a query's result batches —
/// shared by [`diff_fingerprint_sidecar_changed_keys`],
/// [`diff_repair_group_sidecar_changed_keys`], and the stored-output-key
/// read that function's degenerate-comparandum leg falls back to, so the
/// arrow-array downcast lives in exactly one place.
fn extract_delta_keys(batches: &[arrow::record_batch::RecordBatch]) -> Vec<String> {
    let mut keys = Vec::new();
    for batch in batches {
        let Some(col) = batch.column_by_name("delta_key") else {
            continue;
        };
        let Some(arr) = col.as_any().downcast_ref::<arrow::array::StringArray>() else {
            continue;
        };
        for i in 0..arr.len() {
            if !arr.is_null(i) {
                keys.push(arr.value(i).to_string());
            }
        }
    }
    keys
}

/// Write-side: refresh the fingerprint sidecar to match `source_table`'s
/// CURRENT content for `(source_address, projection)`, riding in the SAME
/// backend transaction as `write_group` — the consuming write this refresh
/// is paired with (`docs/specs/sources.md` §"The fingerprint sidecar" —
/// "Transactionality"). Call this AFTER
/// [`diff_fingerprint_sidecar_changed_keys`] has already read the
/// changed-key set the write is about to consume — refreshing first would
/// make a subsequent diff compare the source against itself and observe no
/// changes.
///
/// Gated on `supports_fingerprint_sidecar`, matching
/// [`diff_fingerprint_sidecar_changed_keys`]'s own posture; a target lacking
/// the capability fails loudly rather than being handed DuckDB-flavored SQL
/// it cannot run. The DDL owner is still DuckDB-shaped — see that
/// function's doc comment.
///
/// `model_sql` must be the SAME consuming-model SQL text passed to the
/// paired [`diff_fingerprint_sidecar_changed_keys`] call this refresh
/// follows — it is folded into every refreshed row's stamp
/// ([`compute_fingerprint_sidecar_stamp`]), which is what "self-heals" a
/// stale partition: this upsert runs over every currently-observed key
/// (not just a changed subset), so it unconditionally re-stamps every
/// still-existing row with the current stamp, matching
/// `generate_fingerprint_sidecar_refresh_sql`'s own doc comment.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_fingerprint_sidecar(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    source_key: &[String],
    projection: &FingerprintProjection,
    all_source_columns: &[String],
    model_sql: &str,
    consumer_address: &str,
    write_group: &StatementGroup,
) -> std::result::Result<(), BackendError> {
    if !backend.capabilities().supports_fingerprint_sidecar {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "fingerprint-sidecar refresh for a mutable_snapshot external source (F3)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    let digest_columns = resolve_fingerprint_digest_columns(projection, all_source_columns);
    let identity = fingerprint::projection_identity(projection);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let dialect = maintenance_dialect(backend.dialect());
    let digest_select =
        emit_fingerprint_digest_select(source_table, source_key, &digest_columns, dialect);
    let refresh_sql = ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &stamp,
        &digest_select,
    );
    let gc_sql = ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &digest_select,
    );
    backend
        .execute_write_and_refresh_fingerprint_sidecar(
            &ensure_sql,
            write_group,
            &refresh_sql,
            &gc_sql,
        )
        .await
}

// ── P9: group-grain fingerprint sidecar (the repair family) ────────────
// (`docs/outcomes/20260809-repair-family/phases/09-plan.md`;
// `docs/specs/sources.md` §"The fingerprint sidecar" — "Partition grain";
// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
// over a `mutable_snapshot` source")
//
// Same sidecar table, same stamp/invalidation machinery as the per-row
// grain above — only the partition identity and what one sidecar row
// represents differ. A group-grain partition never collides with a P4
// per-row partition: `repair_group_partition_identity`'s `repair:group=...`
// text can never equal `fingerprint::projection_identity`'s own `cols:...`/
// `full_row` shapes.

/// The repair-scoped sidecar partition identity for a group-grain digest:
/// `model` (via the caller's own `source_address`, unchanged — the
/// partition key stays `(source_address, projection_identity,
/// consumer_address, source_key)`) plus the group-key and digest-column
/// sets, so two different repair cells
/// over the SAME source (a different group key, or a different digest
/// column set) land in different, non-colliding partitions, and neither
/// collides with the per-row [`fingerprint::projection_identity`] partition
/// a P4-driven consumer of the same source might also hold (that always
/// starts `cols:` or is exactly `full_row`; this always starts `repair:`).
fn repair_group_partition_identity(group_key: &[String], digest_columns: &[String]) -> String {
    format!(
        "repair:group={}:digest={}",
        group_key.join(","),
        digest_columns.join(",")
    )
}

/// Build a single-column `delta_key` relation (the same shape
/// [`repair_affected_keys_select`] and the sidecar-diff emitters project)
/// from an already-resolved list of literal key values — the bridge between
/// [`diff_repair_group_sidecar_changed_keys`]'s resolved `Vec<String>` (an
/// executed read, not opaque SQL text) and every downstream repair builder
/// ([`repair_candidate_select`], [`repair_slice_predicate`],
/// [`emit_per_group_recompute`]), which all consume `affected_keys_select`
/// as SQL text regardless of which discovery route produced it.
///
/// An empty `keys` list yields a well-typed EMPTY relation (`WHERE FALSE`),
/// never an invalid `VALUES ()` — a repair with no affected keys this run is
/// a legitimate (if unusual) outcome, not an error.
///
/// The row-set constructor itself — `VALUES (…)` where `dialect` supports
/// one, the portable `SELECT … UNION ALL SELECT …` rewrite GoogleSQL
/// requires otherwise — comes from `smelt_core::build_row_set_table`, the
/// single dialect-aware owner.
pub fn repair_keys_literal_select(keys: &[String], dialect: MaintenanceDialect) -> String {
    if keys.is_empty() {
        return "SELECT CAST(NULL AS VARCHAR) AS delta_key WHERE FALSE".to_string();
    }
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| vec![format!("'{}'", k.replace('\'', "''"))])
        .collect();
    let row_set = smelt_core::build_row_set_table(
        maintenance_dialect_to_backend_type(dialect),
        "__smelt_repair_group_keys",
        &["delta_key"],
        &rows,
    );
    format!("SELECT * FROM {row_set}")
}

/// Map [`MaintenanceDialect`] (the maintenance-statement dialect, three
/// variants) to [`smelt_core::BackendType`] (the row-set owner's dialect
/// parameter) — a 1:1 relabeling, not a lossy collapse: both enumerate
/// exactly DuckDB, Spark, and BigQuery.
fn maintenance_dialect_to_backend_type(dialect: MaintenanceDialect) -> smelt_core::BackendType {
    match dialect {
        MaintenanceDialect::DuckDb => smelt_core::BackendType::DuckDB,
        MaintenanceDialect::Spark => smelt_core::BackendType::Spark,
        MaintenanceDialect::BigQuery => smelt_core::BackendType::BigQuery,
    }
}

/// Read-side: the repair family's group-grain affected-key set for a
/// `MutationProfile::MutableSnapshot` source with no native change feed
/// (P9). Mirrors [`diff_fingerprint_sidecar_changed_keys`]'s shape (ensure
/// the sidecar table, detect staleness, run the emitter-authored diff), but
/// over [`emit_repair_group_sidecar_diff`]'s group-grain digest and a
/// repair-scoped partition identity ([`repair_group_partition_identity`]),
/// and with one further obligation the per-row read has no analogue for:
/// **an absent or stale-stamped comparandum cannot distinguish "a group
/// that vanished" from "a group that never existed"**
/// (`docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"), so for such a run the returned set
/// additionally unions every key currently present in the stored
/// `output_table` — a sound over-approximation that degenerates to a
/// whole-table repair for that one run and self-heals once
/// [`refresh_repair_group_sidecar`] populates a trustworthy comparandum.
///
/// Gated on `supports_fingerprint_sidecar`, matching every other sidecar
/// consumer in this module — a target lacking the capability fails loud
/// (`BackendError::unsupported`) rather than silently falling back to the
/// unsound current-source scan.
#[allow(clippy::too_many_arguments)]
pub async fn diff_repair_group_sidecar_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    output_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    model_sql: &str,
    consumer_address: &str,
) -> std::result::Result<Vec<String>, BackendError> {
    if !backend.capabilities().supports_fingerprint_sidecar {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "group-grain fingerprint-sidecar diff for a mutable_snapshot repair source (P9)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let identity = repair_group_partition_identity(group_key, digest_columns);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);

    let exists_sql = ddl_duckdb::generate_fingerprint_sidecar_partition_exists_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
    );
    let exists_rows = backend.execute_sql(&exists_sql).await?;
    let partition_absent = !exists_rows.iter().any(|batch| batch.num_rows() > 0);

    let stale_check_sql = ddl_duckdb::generate_fingerprint_sidecar_stale_check_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &stamp,
    );
    let stale_rows = backend.execute_sql(&stale_check_sql).await?;
    let has_stale = stale_rows.iter().any(|batch| batch.num_rows() > 0);
    if has_stale {
        tracing::warn!(
            source_address,
            projection_identity = %identity,
            consumer_address,
            "group-grain fingerprint sidecar stamp mismatch detected (model definition or \
             digest inputs changed, or the stored stamp was corrupted); treating the stale \
             partition as absent and widening the affected set to every currently-observed \
             group plus every stored output group"
        );
    }

    let sidecar_table = format!("{schema}.{}", ddl_duckdb::FINGERPRINT_SIDECAR_TABLE_NAME);
    let dialect = maintenance_dialect(backend.dialect());
    let diff_sql = emit_repair_group_sidecar_diff(
        source_table,
        group_key,
        digest_columns,
        &sidecar_table,
        source_address,
        &identity,
        consumer_address,
        &stamp,
        dialect,
    );
    let batches = backend.execute_sql(&diff_sql).await?;
    let mut keys = extract_delta_keys(&batches);

    if partition_absent || has_stale {
        let output_key_columns: Vec<String> = group_key
            .iter()
            .map(|k| format!("{output_table}.{k}"))
            .collect();
        let output_key_expr =
            smelt_logical::maintenance::emit::key_expr_for_columns(&output_key_columns);
        let stored_keys_sql = format!("SELECT {output_key_expr} AS delta_key FROM {output_table}");
        let stored_batches = backend.execute_sql(&stored_keys_sql).await?;
        keys.extend(extract_delta_keys(&stored_batches));
        keys.sort();
        keys.dedup();
    }

    Ok(keys)
}

/// Write-side: refresh the group-grain sidecar partition to match
/// `source_table`'s CURRENT group-grain digests, riding in the SAME backend
/// transaction as `write_group` — the repair's own write. Mirrors
/// [`refresh_fingerprint_sidecar`]'s shape and transactional contract
/// exactly (call AFTER [`diff_repair_group_sidecar_changed_keys`] has
/// already read the affected set this refresh's write is about to
/// consume), reusing the SAME DDL/DML the per-row sidecar uses
/// (`generate_fingerprint_sidecar_table_ddl`,
/// `generate_fingerprint_sidecar_refresh_sql`,
/// `generate_fingerprint_sidecar_gc_sql`) — only the digest select and
/// partition identity differ.
///
/// Populating this on the create/full-refresh path (in addition to every
/// live repair run) is what keeps a model's FIRST incremental repair from
/// taking the absent-comparandum degradation every single time: without an
/// initial populate, every run would find no partition, union in the whole
/// stored output, and never build a trustworthy baseline.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_repair_group_sidecar(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    model_sql: &str,
    consumer_address: &str,
    write_group: &StatementGroup,
) -> std::result::Result<(), BackendError> {
    if !backend.capabilities().supports_fingerprint_sidecar {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "group-grain fingerprint-sidecar refresh for a mutable_snapshot repair source (P9)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    let identity = repair_group_partition_identity(group_key, digest_columns);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let dialect = maintenance_dialect(backend.dialect());
    let digest_select =
        emit_repair_group_digest_select(source_table, group_key, digest_columns, dialect);
    let refresh_sql = ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &stamp,
        &digest_select,
    );
    let gc_sql = ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        schema,
        source_address,
        &identity,
        consumer_address,
        &digest_select,
    );
    backend
        .execute_write_and_refresh_fingerprint_sidecar(
            &ensure_sql,
            write_group,
            &refresh_sql,
            &gc_sql,
        )
        .await
}
