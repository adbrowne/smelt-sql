//! Fingerprint and sidecar-diff emission: the tagged, NULL-safe row and
//! key digest expressions, the digest `SELECT`s built over them, and the
//! sidecar diff / affected-key selects that consume them.

use super::probes::probe_dialect_string_type;
use super::types::*;

// ── Fingerprint sidecar diff (F3, `docs/plans/20260715-composed-axes-
// conditional-maintenance.md` Phase F3; `docs/specs/sources.md` §"The
// fingerprint sidecar") ────────────────────────────────────────────────
//
// The sidecar's own storage DDL/DML (table creation, the upsert-refresh,
// the GC delete) is warehouse-resident bookkeeping, in the same excluded
// class as the reconciliation ledger and the observed-output-delta record
// (`docs/specs/incremental_models.md` §"Statement emission (single
// owner)"'s third exclusion) — it lives in `smelt_state::ddl_duckdb`, not
// here. The DIFF query below is different: unlike the ledger/observed-delta
// bookkeeping, it is not a record of smelt's own run history — it is the
// derived comparison that decides which source keys count as "changed", the
// same kind of maintenance-relevant computation `emit_column_scoped_merge_
// suppressed`'s `IS DISTINCT FROM` guard is. F3 rules it emitter-authored.

/// Tag prefixed onto a NULL column's pre-image before hashing — see
/// [`column_fingerprint_expr`]. A single-character tag with no column
/// content appended, so its pre-image (`'N'`) can never be reproduced by a
/// real column value's own tagged pre-image (which always starts with
/// [`VALUE_TAG`] instead).
const NULL_TAG: &str = "N";

/// Tag prefixed onto a real (non-NULL) column value's pre-image before
/// hashing — see [`column_fingerprint_expr`]. Distinct from [`NULL_TAG`] so
/// no column value, however it stringifies, can ever collide with the NULL
/// pre-image.
const VALUE_TAG: &str = "V";

/// A single column's fingerprint: `sha256` of a tagged pre-image,
/// `'N'` when the column is NULL, `'V' || CAST(col AS VARCHAR)` otherwise.
/// This is the collision-free replacement for the old
/// `COALESCE(CAST(col AS VARCHAR), sentinel)` scheme: that scheme conflated
/// a real value literally equal to the sentinel string with a true NULL
/// (both coalesced to the identical sentinel text). Here NULL and every
/// real value start from structurally disjoint pre-images — `'N'` alone
/// vs. `'V'` followed by (possibly empty, possibly arbitrary) content — so
/// no column content, whatever it contains, can ever produce the NULL
/// pre-image.
///
/// The output is always a fixed-length 64-character hex string (DuckDB's
/// `sha256()` return shape), which is what lets [`concat_varchar_expr`]
/// join multiple columns' fingerprints with no separator at all — see its
/// own doc comment for why fixed-length concatenation removes the
/// separator-collision hazard structurally rather than by convention.
fn column_fingerprint_expr(column: &str, cast_type: &str) -> String {
    format!(
        "sha256(CASE WHEN {column} IS NULL THEN '{NULL_TAG}' ELSE CONCAT('{VALUE_TAG}', CAST({column} AS {cast_type})) END)"
    )
}

/// A row-content fingerprint over one or more DIGEST columns: always the
/// full collision-free construction, single- or multi-column alike. This is
/// a digest-of-digests: each column is hashed independently first via
/// [`column_fingerprint_expr`] into a FIXED-length (64 hex character)
/// output, and only those fixed-length outputs are concatenated — with no
/// separator, because none is needed. The old scheme joined raw
/// (variable-length) `CAST(... AS VARCHAR)` text with a `\u{1}` separator
/// character; since that separator was not escaped within column content, a
/// column value that itself contained a literal `\u{1}` byte could make two
/// genuinely different multi-column tuples reassemble into the identical
/// joined string (e.g. columns `('John\u{1}Smith', 'X')` and `('John',
/// 'Smith\u{1}X')` joined to the same text). Fixed-length concatenation
/// removes this class of bug structurally: every joined component is
/// exactly 64 characters, so there is no byte position at which one
/// column's content could be misread as spanning into an adjacent column's
/// slot, regardless of what that content is.
///
/// This is safe to use unconditionally for a digest because `delta_digest`
/// is never surfaced to a caller as a literal value — it is only ever
/// compared for equality against another digest computed the same way
/// (`IS DISTINCT FROM`). Contrast [`key_expr_for_columns`], which builds
/// the sidecar's KEY expression and — for a single column — must stay a
/// literal, un-hashed value instead; see that function's own doc comment
/// for why.
fn concat_varchar_expr(columns: &[String]) -> String {
    concat_varchar_expr_typed(columns, "VARCHAR")
}

/// [`concat_varchar_expr`], parameterized over the unsized string-cast type
/// name — DuckDB's `VARCHAR` for every existing (DuckDB-only) caller, or the
/// dialect's own type via [`probe_dialect_string_type`] for
/// [`row_fingerprint_expr`]'s dialect-aware probe caller.
fn concat_varchar_expr_typed(columns: &[String], cast_type: &str) -> String {
    let per_column = columns
        .iter()
        .map(|c| column_fingerprint_expr(c, cast_type))
        .collect::<Vec<_>>()
        .join(", ");
    if columns.len() == 1 {
        // Already a single fixed-length sha256 digest — nothing to join.
        per_column
    } else {
        format!("CONCAT({per_column})")
    }
}

/// A whole-row content fingerprint over `columns`: `sha256` of the
/// per-column digest-of-digests concatenation ([`concat_varchar_expr_typed`])
/// — the same construction [`emit_fingerprint_digest_select`] uses for its
/// `delta_digest` column, factored out so
/// [`emit_append_only_posture_probe`] can build the identical row-content
/// hash without re-authoring the hashing SQL. `dialect` selects the
/// unsized string-cast type ([`probe_dialect_string_type`]) — DuckDB's
/// `VARCHAR` or Spark's `STRING` — so the fingerprint is well-formed under
/// either dialect, unlike [`concat_varchar_expr`]'s DuckDB-only default.
pub(crate) fn row_fingerprint_expr(columns: &[String], dialect: MaintenanceDialect) -> String {
    let concatenated = concat_varchar_expr_typed(columns, probe_dialect_string_type(dialect));
    match dialect {
        // GoogleSQL's SHA256 returns BYTES; the row hash is fed straight into a
        // STRING_AGG, so it has to be hex-encoded to stay a STRING.
        MaintenanceDialect::BigQuery => format!("TO_HEX(SHA256({concatenated}))"),
        _ => format!("sha256({concatenated})"),
    }
}

/// The NULL-key sentinel: a KEY column that is truly NULL is coalesced to
/// this fixed marker purely so `delta_key` never violates the sidecar's
/// `source_key VARCHAR NOT NULL` column. Unlike [`NULL_TAG`]/[`VALUE_TAG`],
/// this is NOT collision-free against an adversarial real value — a real
/// source-key column whose content happened to literally equal this marker
/// would be indistinguishable from a true NULL key. That gap is deliberate
/// and narrower in scope than the digest fix: see [`key_expr_for_columns`].
const KEY_NULL_SENTINEL: &str = "\u{2}NULL\u{2}";

/// Builds the sidecar's KEY expression (`delta_key`) over `columns` — the
/// row's identifying key.
///
/// **Single column: stays a literal, un-hashed value.** Unlike the digest
/// expression, `delta_key` is not an opaque comparison-only token: it is
/// surfaced to callers (`smelt_runtime::maintenance_driver::
/// diff_fingerprint_sidecar_changed_keys`'s returned `Vec<String>`) and
/// consumed downstream as a literal predicate value spliced back against
/// the source's own real key column
/// (`emit_delete_insert_delta_restricted`'s `restrict_column IN
/// (delta_keys)`) — the same literal-value contract
/// `smelt_runtime::maintenance_driver::changed_keys_select`'s own
/// `key_expr` upholds (see that function's doc comment for the parallel
/// case). Hashing a single-column key would silently break every consumer
/// that expects `delta_key` to equal the real key's own text. A NULL key
/// column is coalesced to [`KEY_NULL_SENTINEL`] purely to satisfy the
/// sidecar's `NOT NULL` column — narrower than the digest's fix (source
/// identity keys are not expected to be NULL in practice, and the
/// literal-value contract above forecloses hashing NULL away the way the
/// digest does).
///
/// **Multi-column: gets the full collision-free construction.** A
/// composite key has no literal consumer today — no downstream restriction
/// wiring exists for a composite key (`emit_delete_insert_delta_restricted`'s
/// `restrict_column` is always a single physical column) — so there is no
/// contract to preserve, and the composite-key collision the review flagged
/// (two distinct real composite keys silently overwriting the same sidecar
/// row because their old-scheme joined text collided) is worth closing.
///
/// `pub`, not module-private: the repair family's runtime driver
/// (`smelt_runtime::maintenance_driver`) builds the SAME canonical
/// `delta_key` expression over the model's own group-key columns, for the
/// affected-key relation and its `emit_per_group_recompute` joins
/// (`docs/outcomes/20260809-repair-family/phases/09-plan.md`) — one shape,
/// shared by both the sidecar diff and the append-only clamped-scan path,
/// never a second, independently-typed key expression.
pub fn key_expr_for_columns(columns: &[String]) -> String {
    if columns.len() == 1 {
        format!(
            "COALESCE(CAST({} AS VARCHAR), '{KEY_NULL_SENTINEL}')",
            columns[0]
        )
    } else {
        concat_varchar_expr(columns)
    }
}

/// The row-content digest `SELECT` over an external `mutable_snapshot`
/// source (`docs/specs/sources.md` §"The fingerprint sidecar" — "Digest"):
/// `sha256(...)` over `digest_columns` — the caller's already-resolved P4
/// fingerprint projection (`model_properties.md` §"Fingerprint
/// projection"; the source's FULL column list when the projection failed
/// closed to `FullRow`) — keyed by `source_key`. Pure string construction,
/// matching this module's whole-file convention: the caller resolves which
/// columns to digest and which key columns identify a row; this emitter
/// only builds the SQL.
///
/// `dialect` is accepted for signature symmetry with every other emitter in
/// this module; only the DuckDB shape is built today (`sha256()` is a
/// DuckDB built-in scalar function) — a Spark digest-select variant is
/// unbuilt, matching this phase's DuckDB-only sidecar scope. The runtime
/// caller (`smelt_runtime::maintenance_driver`) gates on the backend's
/// dialect before ever reaching this function, so a Spark target fails
/// loud at that call site rather than being handed DuckDB-flavored SQL.
///
/// # Panics
/// Panics if `source_key` or `digest_columns` is empty — a caller with no
/// key to identify rows by, or nothing to digest, has no business building
/// a sidecar digest select at all.
pub fn emit_fingerprint_digest_select(
    source_table: &str,
    source_key: &[String],
    digest_columns: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !source_key.is_empty(),
        "emit_fingerprint_digest_select requires a non-empty source key for {source_table}"
    );
    assert!(
        !digest_columns.is_empty(),
        "emit_fingerprint_digest_select requires a non-empty digest column set for {source_table}"
    );
    let key_expr = key_expr_for_columns(source_key);
    let digest_expr = row_fingerprint_expr(digest_columns, MaintenanceDialect::DuckDb);
    format!("SELECT {key_expr} AS delta_key, {digest_expr} AS delta_digest FROM {source_table}")
}

/// The synthesized external change-feed diff (`docs/specs/sources.md`
/// §"The fingerprint sidecar"): compares `source_table`'s CURRENT
/// `(key, digest)` pairs (via [`emit_fingerprint_digest_select`]) against
/// the sidecar's own stored partition for `(source_address,
/// projection_identity)`, producing the changed-key set a `mutable_snapshot`
/// source's otherwise whole-table delta collapses to.
///
/// A `FULL OUTER JOIN` so three shapes all surface as a changed
/// `delta_key`: a source key with no sidecar row (new — or, on a first run
/// against an unpopulated sidecar, EVERY row, which is exactly the
/// whole-table delta the widen-never-narrow default already produces, with
/// no special-casing needed here); a sidecar row with no source key (the
/// source row was deleted — GC's own trigger, reported via
/// `COALESCE(..., sidecar.source_key)`); and a matched pair whose digests
/// differ (`IS DISTINCT FROM`, the same exact-compare shape every other
/// change-suppression guard in this module uses). A matched pair with equal
/// digests is excluded — never surfaced as a false "changed" result, the
/// digest-soundness oracle's negative leg
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// F1's ruling; `docs/specs/sources.md` §"The fingerprint sidecar" —
/// "Digest" — "the collision-soundness invariant").
///
/// `sidecar_table` is already fully qualified (`schema.table`);
/// `source_address`/`projection_identity`/`consumer_address`/`stamp` are
/// plain string values, escaped here (this emitter, like every other, does
/// its own literal quoting — see `emit_delete_insert_delta_restricted`'s
/// `delta_keys` handling for the same pattern). `consumer_address` is the
/// CONSUMING model's own address (`docs/specs/sources.md` §"The fingerprint
/// sidecar" — "Naming and namespace"): filtering on it, alongside
/// `source_address`/`projection_identity`, is what keeps two consumers of
/// the same source under the same P4 projection from reading each other's
/// comparandum.
///
/// `stamp` (Phase F4,
/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` —
/// "Sidecar invalidation") is the freshly computed identity stamp
/// (`smelt_runtime::maintenance_driver::compute_fingerprint_sidecar_stamp`
/// — digest-construction version, this same `projection_identity`, and the
/// consuming model's own SQL provenance combined). The sidecar-side
/// subquery filters on `stamp = '...'` in addition to `source_address`/
/// `projection_identity`: a stored row whose stamp does not match is
/// excluded from the comparison entirely, so it never joins against the
/// current source content — structurally identical to that key having no
/// sidecar row at all. This is the mechanism that makes an invalidated
/// partition (a model-definition edit, a P4 projection change reusing the
/// same identity text is impossible by construction, or a digest-version
/// bump) degrade to exactly the same whole-table delta an absent sidecar
/// already produces above — never a narrower, partially-trusted
/// comparison, and never a silent skip.
#[allow(clippy::too_many_arguments)]
pub fn emit_fingerprint_sidecar_diff(
    source_table: &str,
    source_key: &[String],
    digest_columns: &[String],
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    consumer_address: &str,
    stamp: &str,
    dialect: MaintenanceDialect,
) -> String {
    let digest_select =
        emit_fingerprint_digest_select(source_table, source_key, digest_columns, dialect);
    sidecar_diff_over_digest_select(
        &digest_select,
        sidecar_table,
        source_address,
        projection_identity,
        consumer_address,
        stamp,
    )
}

/// Shared `FULL OUTER JOIN` shape both [`emit_fingerprint_sidecar_diff`]
/// (per-row grain) and [`emit_repair_group_sidecar_diff`] (group grain, P9)
/// build over their own `digest_select` — the comparison logic (three-way
/// new/deleted/changed classification, the stamp filter) is identical at
/// either grain; only what `digest_select` projects one `delta_key`/
/// `delta_digest` pair PER (a source row, or a source-derived output group)
/// differs. See [`emit_fingerprint_sidecar_diff`]'s own doc comment for the
/// full rationale — this helper exists purely to keep that rationale in one
/// place rather than duplicated across two near-identical `format!` bodies.
fn sidecar_diff_over_digest_select(
    digest_select: &str,
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    consumer_address: &str,
    stamp: &str,
) -> String {
    let source_address_lit = source_address.replace('\'', "''");
    let projection_identity_lit = projection_identity.replace('\'', "''");
    let consumer_address_lit = consumer_address.replace('\'', "''");
    let stamp_lit = stamp.replace('\'', "''");
    format!(
        "SELECT COALESCE(__smelt_src.delta_key, __smelt_sidecar.source_key) AS delta_key \
         FROM ({digest_select}) AS __smelt_src \
         FULL OUTER JOIN (SELECT source_key, digest FROM {sidecar_table} \
         WHERE source_address = '{source_address_lit}' AND projection_identity = '{projection_identity_lit}' \
         AND consumer_address = '{consumer_address_lit}' AND stamp = '{stamp_lit}') \
         AS __smelt_sidecar ON __smelt_src.delta_key = __smelt_sidecar.source_key \
         WHERE __smelt_sidecar.source_key IS NULL \
         OR __smelt_src.delta_key IS NULL \
         OR __smelt_src.delta_digest IS DISTINCT FROM __smelt_sidecar.digest"
    )
}

/// The repair family's group-grain digest `SELECT`
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "Partition grain";
/// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"): one row per `group_key` value,
/// projecting the same canonical `delta_key` expression
/// [`emit_fingerprint_digest_select`] builds ([`key_expr_for_columns`] over
/// `group_key`), paired with an **order-insensitive** digest over that
/// group's own contributing source rows.
///
/// Each contributing row is hashed independently first, via the same
/// tagged, NULL-safe, fixed-length per-row digest [`concat_varchar_expr`]
/// builds for the per-row sidecar (`sha256(...)` over `digest_columns`);
/// `hash(...)` (DuckDB's scalar hash, `UBIGINT`) turns that fixed-length
/// hex digest into an integer, and `bit_xor(...)` combines every row's
/// integer digest within the group — XOR is commutative and associative, so
/// the group's digest does not depend on the order its rows are read in
/// (the same content in a different row order digests identically), while
/// removing, adding, or changing any one row's content still flips bits in
/// the combined result (a collision needs two DISTINCT per-row digest sets
/// to XOR to the same value, no likelier than the per-row sidecar's own
/// assumed SHA-256 collision-soundness invariant `sources.md` §"The
/// fingerprint sidecar" — "Digest" already relies on).
///
/// `dialect` is accepted for signature symmetry with
/// [`emit_fingerprint_digest_select`]; only the DuckDB shape (`sha256`,
/// `hash`, `bit_xor` are all DuckDB built-ins) is built today, matching this
/// phase's DuckDB-only scope.
///
/// # Panics
/// Panics if `group_key` or `digest_columns` is empty — mirrors
/// [`emit_fingerprint_digest_select`]'s own contract.
pub fn emit_repair_group_digest_select(
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !group_key.is_empty(),
        "emit_repair_group_digest_select requires a non-empty group key for {source_table}"
    );
    assert!(
        !digest_columns.is_empty(),
        "emit_repair_group_digest_select requires a non-empty digest column set for \
         {source_table}"
    );
    let key_expr = key_expr_for_columns(group_key);
    let group_by_list = group_key.join(", ");
    let row_digest_expr = concat_varchar_expr(digest_columns);
    format!(
        "SELECT {key_expr} AS delta_key, CAST(bit_xor(hash(sha256({row_digest_expr}))) AS \
         VARCHAR) AS delta_digest FROM {source_table} GROUP BY {group_by_list}"
    )
}

/// The repair family's group-grain counterpart of
/// [`emit_fingerprint_sidecar_diff`] (P9,
/// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"): the same `FULL OUTER JOIN` diff
/// shape, over [`emit_repair_group_digest_select`]'s group-grain digest
/// instead of the per-row one — so a group whose entire contribution
/// departed the source still surfaces via the diff's "sidecar row with no
/// matching source key" leg (`__smelt_src.delta_key IS NULL`), even though
/// no source row survives to name it.
#[allow(clippy::too_many_arguments)]
pub fn emit_repair_group_sidecar_diff(
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    consumer_address: &str,
    stamp: &str,
    dialect: MaintenanceDialect,
) -> String {
    let digest_select =
        emit_repair_group_digest_select(source_table, group_key, digest_columns, dialect);
    sidecar_diff_over_digest_select(
        &digest_select,
        sidecar_table,
        source_address,
        projection_identity,
        consumer_address,
        stamp,
    )
}

/// A key-addressed model edge's affected-keys relation
/// (`docs/specs/incremental_models.md` §"Upstream model edges"): the
/// downstream's own key columns (`KeyScope::keys`), distinct, for every
/// upstream row whose own `KeyedUpsert` key (`upstream_keys`) is one of the
/// already-resolved `changed_keys` — the changed-key set the group-grain
/// fingerprint sidecar diff over the upstream's output table discovered
/// (`diff_repair_group_sidecar_changed_keys`, `smelt-runtime`). This is the
/// key-correspondence projection: an upstream key that changed does not
/// necessarily equal the downstream's own key column set, so the relation
/// re-selects the downstream's key expression over the upstream table rather
/// than reusing the changed keys directly.
///
/// Same `key_expr_for_columns` canonicalisation
/// [`repair_affected_keys_select`]/[`repair_candidate_select`] (`smelt-
/// runtime`) use for the resulting `delta_key` column, so this relation
/// composes into the same repair-family candidate-select/write emitters
/// unchanged — only how the affected-key relation itself is discovered
/// differs from the ordinary clamped-scan repair path.
///
/// `changed_keys` is a literal `VARCHAR` value list (already resolved by the
/// caller's sidecar-diff read, not opaque SQL) — the same shape
/// [`super::super::maintenance_driver`]'s `repair_keys_literal_select`-style
/// callers pass. An empty `changed_keys` yields a well-typed EMPTY relation
/// (`WHERE FALSE`), never an unrestricted `SELECT DISTINCT`: a run
/// discovering no changed upstream keys touches nothing.
///
/// `dialect` is accepted for signature symmetry with this module's other
/// repair-family emitters; only the DuckDB shape is built today, matching
/// this phase's DuckDB-only discovery-route scope.
///
/// # Panics
/// Panics if `upstream_keys` or `downstream_keys` is empty — mirrors
/// [`emit_repair_group_digest_select`]'s own contract.
pub fn emit_key_addressed_affected_keys_select(
    upstream_table: &str,
    upstream_keys: &[String],
    downstream_keys: &[String],
    changed_keys: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !upstream_keys.is_empty(),
        "emit_key_addressed_affected_keys_select requires a non-empty upstream key for \
         {upstream_table}"
    );
    assert!(
        !downstream_keys.is_empty(),
        "emit_key_addressed_affected_keys_select requires a non-empty downstream key for \
         {upstream_table}"
    );
    let downstream_key_expr = key_expr_for_columns(downstream_keys);
    if changed_keys.is_empty() {
        return format!(
            "SELECT {downstream_key_expr} AS delta_key FROM {upstream_table} WHERE FALSE"
        );
    }
    let upstream_key_expr = key_expr_for_columns(upstream_keys);
    let literals = changed_keys
        .iter()
        .map(|k| format!("'{}'", k.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT DISTINCT {downstream_key_expr} AS delta_key FROM {upstream_table} WHERE \
         {upstream_key_expr} IN ({literals})"
    )
}

#[cfg(test)]
mod fingerprint_sidecar_tests {
    use super::*;

    /// Run `emit_fingerprint_digest_select` for a single-row `source_table`
    /// (a derived-table expression, e.g. `(SELECT 1 AS id, 'x' AS val)")
    /// against a real DuckDB and return the resulting `delta_digest` value
    /// — used by the two collision-regression tests below to prove the
    /// FIX's actual SQL output against real DuckDB semantics (NULL
    /// handling, `chr()`, `CONCAT`), not merely against string-literal
    /// expectations of what DuckDB is assumed to do.
    fn digest_for_source(
        conn: &duckdb::Connection,
        source_table: &str,
        source_key: &[String],
        digest_columns: &[String],
    ) -> String {
        let sql = emit_fingerprint_digest_select(
            source_table,
            source_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        conn.query_row(&sql, [], |row| row.get::<_, String>(1))
            .expect("digest select query")
    }

    /// Same as [`digest_for_source`] but returns the `delta_key` column
    /// instead — used by the composite-source-key collision regression
    /// test below.
    fn key_for_source(
        conn: &duckdb::Connection,
        source_table: &str,
        source_key: &[String],
        digest_columns: &[String],
    ) -> String {
        let sql = emit_fingerprint_digest_select(
            source_table,
            source_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        conn.query_row(&sql, [], |row| row.get::<_, String>(0))
            .expect("key select query")
    }

    #[test]
    fn digest_select_single_column_key_and_digest() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // The single-column KEY stays literal (`COALESCE(CAST(... AS
        // VARCHAR), sentinel)`) — it is surfaced downstream as a real
        // predicate value, unlike the digest, which is always the full
        // tagged-hash construction (see `key_expr_for_columns`'s doc
        // comment for why the two differ).
        assert_eq!(
            sql,
            "SELECT COALESCE(CAST(user_id AS VARCHAR), '\u{2}NULL\u{2}') AS delta_key, \
             sha256(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END)) AS delta_digest FROM raw.dim_users"
        );
    }

    #[test]
    fn digest_select_multi_column_key_and_digest_concatenates() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["tenant_id".to_string(), "user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Each column is hashed to a fixed-length digest FIRST, and only
        // those fixed-length digests are concatenated — no separator, since
        // fixed-length components have no boundary to confuse.
        assert!(sql.contains(
            "CONCAT(sha256(CASE WHEN tenant_id IS NULL THEN 'N' ELSE CONCAT('V', CAST(tenant_id \
             AS VARCHAR)) END), sha256(CASE WHEN user_id IS NULL THEN 'N' ELSE CONCAT('V', CAST(\
             user_id AS VARCHAR)) END)) AS delta_key"
        ));
        assert!(sql.contains(
            "sha256(CONCAT(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END), sha256(CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier \
             AS VARCHAR)) END))) AS delta_digest"
        ));
    }

    /// Regression for the NULL-vs-empty-string digest collision (the first
    /// bug this fingerprint scheme had): DuckDB's `CONCAT` silently drops
    /// NULL arguments, so before any fix at all, `CONCAT(NULL, sep, 'x')`
    /// and `CONCAT('', sep, 'x')` produced the identical string (and
    /// therefore the identical digest) — a false-negative "unchanged"
    /// verdict for a row whose projected value went from empty string to
    /// NULL (or vice versa). The tagged pre-image construction rules this
    /// out structurally: a NULL column's pre-image is the bare tag `'N'`,
    /// disjoint from EVERY real value's pre-image (`'V' || content`,
    /// including the empty string, `'V'`), so the two can never coincide.
    #[test]
    fn digest_select_distinguishes_null_from_empty_string_in_multi_column_projection() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Each column branches on its own NULL-ness independently, so a
        // NULL `name` renders as the `'N'` tag branch, structurally
        // distinct from the `'V'`-tagged empty-string branch — not simply
        // vanishing from a CONCAT the way a dropped NULL argument would.
        assert!(sql.contains(
            "CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS VARCHAR)) END"
        ));
        assert!(sql.contains(
            "CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier AS VARCHAR)) END"
        ));
    }

    /// Regression for the NULL-digest crash (the second bug this
    /// fingerprint scheme had): before any fix, a single-column projection
    /// built `sha256(CAST(col AS VARCHAR))` directly, so a NULL projected
    /// value produced `sha256(NULL) = NULL` in DuckDB — which then violated
    /// the sidecar's `NOT NULL digest` column constraint on upsert. The
    /// emitted digest expression must never let a NULL value reach
    /// `sha256` un-tagged, for both the single- and multi-column shapes.
    #[test]
    fn digest_select_single_column_never_feeds_sha256_a_bare_null() {
        let single = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert!(single.contains(
            "sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS VARCHAR)) END)"
        ));
        assert!(!single.contains("sha256(CAST(name AS VARCHAR))"));

        let multi = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Every column reaching the hash must be wrapped in the tagged
        // CASE — none of the bare `CAST(... AS VARCHAR)` forms may appear
        // unwrapped, and no raw `CONCAT(CAST(` (the old un-hashed
        // multi-column join shape) may appear either.
        assert!(!multi.contains("sha256(CAST("));
        assert!(!multi.contains("CONCAT(CAST("));
    }

    /// Regression for the separator-collision bug found in a follow-up
    /// review: the earlier fix joined raw (unescaped) column text with a
    /// `\u{1}` separator, so a column value that itself contained a literal
    /// `\u{1}` byte could make two DISTINCT multi-column tuples reassemble
    /// into the identical joined string before hashing —
    /// `('John\u{1}Smith', 'X')` and `('John', 'Smith\u{1}X')` both joined
    /// to `John\u{1}Smith\u{1}X`. Confirmed empirically against a real
    /// DuckDB: this computes the ACTUAL digest SQL's result for both
    /// tuples and asserts they differ, proving the fixed-length
    /// digest-of-digests construction never lets one column's content
    /// bleed across a boundary into another, regardless of what that
    /// content contains.
    #[test]
    fn digest_distinguishes_tuples_that_collided_under_the_old_separator_scheme() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["id".to_string()];
        let digest_columns = vec!["name".to_string(), "tier".to_string()];

        // Tuple A: `name` contains a literal SOH (`\u{1}`) byte before
        // "Smith".
        let digest_a = digest_for_source(
            &conn,
            "(SELECT 1 AS id, 'John' || chr(1) || 'Smith' AS name, 'X' AS tier)",
            &source_key,
            &digest_columns,
        );
        // Tuple B: a DIFFERENT (name, tier) pair whose old-scheme joined
        // text was byte-identical to tuple A's:
        // `'John' + SEP + 'Smith' + SEP + 'X'`.
        let digest_b = digest_for_source(
            &conn,
            "(SELECT 2 AS id, 'John' AS name, 'Smith' || chr(1) || 'X' AS tier)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            digest_a, digest_b,
            "two genuinely different (name, tier) tuples must never hash identically, even when \
             a column's own content contains the old separator byte"
        );
    }

    /// Regression for the sentinel-collision bug found in the same
    /// follow-up review: the earlier fix coalesced a NULL column to the
    /// fixed sentinel string `\u{2}NULL\u{2}`, so a REAL column value that
    /// happened to literally equal that sentinel text was indistinguishable
    /// from a true NULL of the same row shape. Confirmed empirically
    /// against a real DuckDB: computes the actual digest for a true-NULL
    /// row and for a row whose value is literally the old sentinel text,
    /// and asserts they differ.
    #[test]
    fn digest_distinguishes_a_real_value_equal_to_the_old_sentinel_from_a_true_null() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["id".to_string()];
        let digest_columns = vec!["val".to_string()];

        let digest_null = digest_for_source(
            &conn,
            "(SELECT 1 AS id, CAST(NULL AS VARCHAR) AS val)",
            &source_key,
            &digest_columns,
        );
        let digest_sentinel_lookalike = digest_for_source(
            &conn,
            "(SELECT 2 AS id, (chr(2) || 'NULL' || chr(2)) AS val)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            digest_null, digest_sentinel_lookalike,
            "a real column value equal to the old NULL sentinel must never hash identically to a \
             true NULL"
        );
    }

    /// Regression for the composite `source_key` half of the
    /// separator-collision bug: [`key_expr_for_columns`] reuses
    /// [`concat_varchar_expr`] for a MULTI-column key (only a single-column
    /// key stays literal — see that function's doc comment), so a composite
    /// key is exposed to the exact same old-scheme collision the digest
    /// was. This is a real correctness hazard beyond a false "unchanged"
    /// verdict: two distinct real composite source keys reassembling to the
    /// SAME `delta_key` string would conflate onto the SAME sidecar row
    /// (`source_key` is part of the sidecar's own primary key), silently
    /// overwriting one key's stored digest with the other's. Confirmed
    /// empirically against a real DuckDB: computes the actual `delta_key`
    /// for two engineered-to-collide composite keys and asserts they
    /// differ.
    #[test]
    fn composite_source_key_distinguishes_tuples_that_collided_under_the_old_separator_scheme() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["tenant".to_string(), "user".to_string()];
        let digest_columns = vec!["val".to_string()];

        let key_a = key_for_source(
            &conn,
            "(SELECT 'John' || chr(1) || 'Smith' AS tenant, 'X' AS user, 'v' AS val)",
            &source_key,
            &digest_columns,
        );
        let key_b = key_for_source(
            &conn,
            "(SELECT 'John' AS tenant, 'Smith' || chr(1) || 'X' AS user, 'v' AS val)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            key_a, key_b,
            "two genuinely different composite source keys must never produce the same \
             delta_key, even when a key column's own content contains the old separator byte"
        );
    }

    #[test]
    #[should_panic(expected = "non-empty source key")]
    fn digest_select_panics_on_empty_source_key() {
        emit_fingerprint_digest_select(
            "raw.dim_users",
            &[],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    #[should_panic(expected = "non-empty digest column set")]
    fn digest_select_panics_on_empty_digest_columns() {
        emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &[],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    fn sidecar_diff_full_outer_joins_source_against_sidecar_partition() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim_users",
            "cols:name,tier",
            "smelt.models.consumer_a",
            "v1:cols:name,tier:sha256:deadbeef",
            MaintenanceDialect::DuckDb,
        );
        assert!(sql.contains("FULL OUTER JOIN"));
        assert!(sql.contains("FROM main._smelt_fingerprint_sidecar"));
        assert!(sql.contains("source_address = 'smelt.sources.dim_users'"));
        assert!(sql.contains("projection_identity = 'cols:name,tier'"));
        assert!(sql.contains("consumer_address = 'smelt.models.consumer_a'"));
        assert!(sql.contains("stamp = 'v1:cols:name,tier:sha256:deadbeef'"));
        assert!(sql.contains("__smelt_src.delta_digest IS DISTINCT FROM __smelt_sidecar.digest"));
        assert!(sql.contains("__smelt_sidecar.source_key IS NULL"));
        assert!(sql.contains("__smelt_src.delta_key IS NULL"));
        assert!(sql.contains(
            "SELECT COALESCE(CAST(user_id AS VARCHAR), '\u{2}NULL\u{2}') AS delta_key, \
             sha256(CONCAT(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END), sha256(CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier \
             AS VARCHAR)) END))) AS delta_digest FROM raw.dim_users"
        ));
    }

    #[test]
    fn sidecar_diff_escapes_single_quotes_in_literals() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim's_users",
            "cols:name",
            "smelt.models.consumer's_a",
            "stamp's",
            MaintenanceDialect::DuckDb,
        );
        assert!(sql.contains("source_address = 'smelt.sources.dim''s_users'"));
        assert!(sql.contains("consumer_address = 'smelt.models.consumer''s_a'"));
        assert!(sql.contains("stamp = 'stamp''s'"));
    }

    /// Phase F4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// — "Sidecar invalidation"): a stale-stamped row must never be joined
    /// against the current source content — the `stamp = '...'` filter
    /// excludes it from the sidecar-side subquery regardless of
    /// `source_address`/`projection_identity` matching, structurally
    /// identical to that key having no sidecar row at all.
    #[test]
    fn sidecar_diff_stamp_filter_excludes_mismatched_rows_from_the_comparison() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim_users",
            "cols:name",
            "smelt.models.consumer_a",
            "v2:cols:name:sha256:newhash",
            MaintenanceDialect::DuckDb,
        );
        // The sidecar-side subquery must filter on the CURRENT stamp only —
        // a row stamped under any other value is never a candidate match.
        assert!(sql.contains(
            "WHERE source_address = 'smelt.sources.dim_users' AND projection_identity = \
             'cols:name' AND consumer_address = 'smelt.models.consumer_a' AND stamp = \
             'v2:cols:name:sha256:newhash'"
        ));
    }

    /// Run [`emit_repair_group_digest_select`] over `source_table` (a
    /// derived-table expression) against a real DuckDB and return the
    /// `(delta_key, delta_digest)` pairs it produces, sorted by key — used
    /// by the group-digest order-insensitivity and vanished-group tests
    /// below to prove the FIX's actual SQL output against real DuckDB
    /// semantics, not merely string-literal expectations.
    fn group_digests(
        conn: &duckdb::Connection,
        source_table: &str,
        group_key: &[String],
        digest_columns: &[String],
    ) -> Vec<(String, String)> {
        let sql = emit_repair_group_digest_select(
            source_table,
            group_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        let mut stmt = conn.prepare(&sql).expect("prepare group digest select");
        let mut rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query group digest select")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect group digest rows");
        rows.sort();
        rows
    }

    /// P9 test 1 (`docs/outcomes/20260809-repair-family/phases/09-plan.md`):
    /// the group digest is an order-insensitive aggregate — inserting the
    /// same group's rows in a different order must not change its digest —
    /// while removing one of the group's rows must.
    #[test]
    fn repair_group_digest_select_is_order_insensitive() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let group_key = vec!["customer_id".to_string()];
        let digest_columns = vec!["amount".to_string()];

        let forward = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 10), (1, 20), (1, 30)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        let shuffled = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 30), (1, 10), (1, 20)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        assert_eq!(
            forward, shuffled,
            "the same group's rows in a different order must digest identically"
        );

        let one_row_deleted = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 10), (1, 20)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        assert_ne!(
            forward, one_row_deleted,
            "deleting one of the group's rows must change its digest"
        );
    }

    /// P9 test 2: the group-grain sidecar diff over a group-grain partition
    /// reports a group present in the sidecar and absent from the source —
    /// the `__smelt_src.delta_key IS NULL` leg — with the vanished group's
    /// key value intact, proving a wholly-deleted group is still
    /// discoverable via the stored comparandum.
    #[test]
    fn repair_group_digest_diff_reports_a_vanished_group() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_table = "(SELECT * FROM (VALUES (1, 10)) AS t(customer_id, amount))";
        let group_key = vec!["customer_id".to_string()];
        let digest_columns = vec!["amount".to_string()];

        // Customer 1's digest under the CURRENT source content — seeded
        // into the sidecar as an already-matching comparandum, so it must
        // NOT surface as changed; only customer 2 (present in the sidecar,
        // absent from the source) should.
        let customer_1_digest = group_digests(&conn, source_table, &group_key, &digest_columns)
            .into_iter()
            .find(|(key, _)| key == "1")
            .expect("customer 1's digest")
            .1;
        conn.execute_batch(&format!(
            "CREATE TABLE sidecar (source_address VARCHAR, projection_identity VARCHAR, \
             consumer_address VARCHAR, source_key VARCHAR, digest VARCHAR, stamp VARCHAR); \
             INSERT INTO sidecar VALUES \
             ('src', 'repair:group=customer_id:digest=amount', 'smelt.models.consumer_a', '1', \
             '{customer_1_digest}', 'stamp1'), \
             ('src', 'repair:group=customer_id:digest=amount', 'smelt.models.consumer_a', '2', \
             'stale-digest-for-vanished-group', 'stamp1');"
        ))
        .expect("seed sidecar: customer 1 matches current content, customer 2 has vanished");

        let sql = emit_repair_group_sidecar_diff(
            source_table,
            &group_key,
            &digest_columns,
            "sidecar",
            "src",
            "repair:group=customer_id:digest=amount",
            "smelt.models.consumer_a",
            "stamp1",
            MaintenanceDialect::DuckDb,
        );
        let mut stmt = conn.prepare(&sql).expect("prepare group sidecar diff");
        let keys: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query group sidecar diff")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect diff keys");
        assert_eq!(
            keys,
            vec!["2".to_string()],
            "customer 2's group vanished entirely from the source — the diff must still report \
             it, sourced from the sidecar's own stored comparandum, while customer 1's unchanged \
             group must not surface: {keys:?}"
        );
    }
}
