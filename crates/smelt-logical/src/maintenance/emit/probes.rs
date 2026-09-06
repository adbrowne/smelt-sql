//! Property probes and posture snapshots: the violation-probe `SELECT`s a
//! run executes to check a declared property still holds, plus the
//! append-only baseline snapshot and source-mutation fingerprint.

use super::fingerprint::row_fingerprint_expr;
use super::types::*;

/// The out-of-slice match probe for a **checked** route-3 (recurrence-
/// bounded, declared `r`) merge (`docs/specs/incremental_models.md`
/// §"Key temporal locality", route 3): a read-only query the caller
/// (`smelt-runtime`'s `maintenance_driver`) executes and inspects the
/// result of *before* running the merge action, so a violation is caught
/// without ever writing to the target — "the whole transaction rolls
/// back" trivially, since nothing was written yet.
///
/// Returns one row with two columns:
/// - `violation_count` (`BIGINT`) — the number of distinct keys the step's
///   own delta shares with a stored target row whose partition column lies
///   *before* the slice's lower bound (`slice_lower`) — exactly the
///   "matched (or would duplicate) a stored key outside the slice" check
///   the spec names.
/// - `sample_keys` (`VARCHAR`, `NULL` when `violation_count` is `0`) — up
///   to 5 comma-joined key tuples, for the `KeyedRecurrenceBoundViolated`
///   diagnostic's "sample keys" obligation.
///
/// `slice_lower` is the same concrete lower-bound literal the merge's own
/// `TargetSlicePredicate::Range` uses (the step's own partition value,
/// widened backward by the declared `r` plus margins) — this emitter does
/// no date arithmetic of its own, matching every other emitter in this
/// module.
pub fn emit_recurrence_bound_probe(
    schema_table: &str,
    key: &[String],
    partition_column: &str,
    delta_select: &str,
    slice_lower: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let cast_type = probe_dialect_string_type(dialect);
    let key_list = key.join(", ");
    let join_cond = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_concat = key
        .iter()
        .map(|k| format!("CAST(target.{k} AS {cast_type})"))
        .collect::<Vec<_>>()
        .join(" || '|' || ");
    let safe_lower = slice_lower.replace('\'', "''");
    let violations_select = format!(
        "SELECT DISTINCT {key_concat} AS violation_key \
         FROM {schema_table} AS target \
         JOIN (SELECT DISTINCT {key_list} FROM ({delta_select})) AS delta ON {join_cond} \
         WHERE target.{partition_column} < '{safe_lower}'"
    );
    let sql = wrap_violation_probe("__recurrence_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
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

/// The unsized string-cast type name for a probe's key-display expression:
/// DuckDB accepts an unsized `VARCHAR`; Spark SQL requires a length on
/// `VARCHAR` (`DATATYPE_MISSING_SIZE`), so its unsized string type is
/// `STRING`. Confirmed live against Spark
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5).
pub(crate) fn probe_dialect_string_type(dialect: MaintenanceDialect) -> &'static str {
    match dialect {
        MaintenanceDialect::DuckDb => "VARCHAR",
        MaintenanceDialect::Spark => "STRING",
        // GoogleSQL has no VARCHAR at all (`Type not found: VARCHAR`); its
        // unsized string type is STRING. Confirmed live (scripts/bigquery-probe3.sh).
        MaintenanceDialect::BigQuery => "STRING",
    }
}

/// The "join up to 5 sampled `violation_key` values into one string"
/// aggregate: DuckDB has `STRING_AGG`; Spark SQL has no `STRING_AGG`, so the
/// equivalent join-aggregate is `CONCAT_WS(', ', COLLECT_LIST(...))`.
/// Shared by every probe emitter in this module (`docs/outcomes/
/// 20260809-probe-backed-facts/phases/02-plan.md` — "Design constraint: one
/// probe result shape").
fn probe_dialect_sample_agg(dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb => "STRING_AGG(violation_key, ', ')".to_string(),
        MaintenanceDialect::Spark => "CONCAT_WS(', ', COLLECT_LIST(violation_key))".to_string(),
        // GoogleSQL has STRING_AGG with the same shape as DuckDB's.
        // Confirmed live (scripts/bigquery-probe3.sh).
        MaintenanceDialect::BigQuery => "STRING_AGG(violation_key, ', ')".to_string(),
    }
}

/// Wraps a caller-supplied `violations_select` — any `SELECT` projecting a
/// `violation_key` column, one row per offending identifier — into the
/// canonical one-row probe result every emitter in this module returns:
/// `violation_count` (the number of offending rows) and `sample_keys` (up to
/// 5 comma-joined offending identifiers, `NULL` when `violation_count` is
/// `0`). `cte_name` lets each caller pick its own descriptive CTE alias
/// (`docs/specs/model_properties.md` §"Probe obligation" — "a probe's answer
/// is a single `violation_count`/`sample_keys` row").
fn wrap_violation_probe(
    cte_name: &str,
    violations_select: &str,
    dialect: MaintenanceDialect,
) -> String {
    let sample_expr = probe_dialect_sample_agg(dialect);
    format!(
        "WITH {cte_name} AS ({violations_select}) \
         SELECT COUNT(*) AS violation_count, \
                (SELECT {sample_expr} FROM \
                 (SELECT violation_key FROM {cte_name} LIMIT 5) AS __sample) \
                 AS sample_keys \
         FROM {cte_name}"
    )
}

/// A key's display expression for a probe's `violation_key` column: each
/// column CAST to the dialect's unsized string type and pipe-concatenated —
/// the same composite-key shape [`emit_recurrence_bound_probe`] uses,
/// factored out so the four declaration probes below share it.
fn probe_key_display_expr(columns: &[String], cast_type: &str) -> String {
    columns
        .iter()
        .map(|c| format!("CAST({c} AS {cast_type})"))
        .collect::<Vec<_>>()
        .join(" || '|' || ")
}

/// The referential-integrity count-preservation tripwire
/// (`docs/specs/sources.md` §"Referential integrity"; `model_properties.md`
/// §"Skeleton-source closure" P1, row-preservation conjunct 4): a read-only
/// query the caller (`smelt-runtime`) executes and inspects *before*
/// trusting an inner-join enrichment recompute a declared
/// `referential_integrity` world-fact licensed, so a violation is caught
/// without ever writing to the target — "the whole transaction rolls back"
/// trivially, the same shape [`emit_recurrence_bound_probe`] uses for
/// route 3's out-of-slice match probe.
///
/// Returns one row with two columns:
/// - `driving_count` (`BIGINT`) — the row count of `driving_select` (the
///   fact side alone, scoped to the touched region) — the count a
///   row-preserving join must not fall short of.
/// - `enriched_count` (`BIGINT`) — the row count of `enriched_select` (the
///   SAME region's inner-join enrichment recompute). `enriched_count <
///   driving_count` disproves the declared `referential_integrity` — some
///   driving row's join key had no match in the dimension, so the inner
///   join silently dropped it — and the caller fails the run loudly
///   (`SourceCountPreservationViolated`) rather than trusting the
///   declaration's licensed technique against a stale or simply wrong
///   fact.
///
/// `driving_select`/`enriched_select` are the caller's own already-compiled
/// `SELECT`s (scoped to the touched region); this emitter does no scoping
/// of its own, matching every other function in this module.
pub fn emit_count_preservation_probe(
    driving_select: &str,
    enriched_select: &str,
) -> MaintenanceStatement {
    let sql = format!(
        "SELECT (SELECT COUNT(*) FROM ({driving_select}) AS __smelt_driving) AS driving_count, \
         (SELECT COUNT(*) FROM ({enriched_select}) AS __smelt_enriched) AS enriched_count"
    );
    MaintenanceStatement::new(sql)
}

/// Find the `SelectStmt` whose own `FROM` clause carries a join against
/// `enrichment_source` — either `select` itself, or, when `select`'s `FROM`
/// is exactly one derived-table source with no joins of its own AND that
/// source is aliased [`smelt_dialect::TYPE_CAST_WRAP_ALIAS`] (the exact
/// marker `wrap_with_type_casts` emits), the `SelectStmt` inside that
/// derived table. This widening exists to see through *that one shape* — a
/// type-cast wrap — and no other: gating on the wrap's own alias, rather
/// than on "any single non-joined derived-table source", is what keeps a
/// user's own legitimate subquery (`SELECT ... FROM (SELECT ... JOIN ...)
/// sub WHERE ...`) from being mistaken for a wrapped body. Matching that
/// shape by accident would have the caller build its driving/enriched
/// selects from the INNER select — silently discarding the OUTER select's
/// own `WHERE`, verifying count-preservation over a different row set than
/// the model actually writes (`docs/plans/20260819-source-derived-projection.md`
/// Phase 5 review finding).
///
/// Recurses through that single shape only, and — matching the "a type-cast
/// wrap nests once, never repeatedly" contract literally, not just in
/// prose — only ever one level deep: the recursive call below is not
/// itself recursive, so a doubly-wrapped body still fails closed. This is
/// structural navigation of the one parse tree
/// `emit_count_preservation_probe_from_body` already built from `body_sql`,
/// over text ranges that remain valid offsets into that same string — never
/// a second parse of a separately reconstructed substring.
///
/// Returns `None` (fail-closed, matching the caller) when `select`'s `FROM`
/// has joins that don't match `enrichment_source` (a genuine miss — no
/// deeper source could still contain the join the caller is scoped to),
/// when the single non-joined source isn't a derived table at all, or when
/// that derived table isn't aliased the wrap marker.
fn select_with_enrichment_join(
    select: &smelt_parser::SelectStmt,
    enrichment_source: &str,
) -> Option<smelt_parser::SelectStmt> {
    let last_segment = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    let target_last = last_segment(enrichment_source);
    let matches = |from: &smelt_parser::FromClause| {
        from.joins().any(|join| {
            join.table_ref()
                .and_then(|table_ref| table_ref.bare_path_text())
                .is_some_and(|path| path == enrichment_source || last_segment(&path) == target_last)
        })
    };

    let from_clause = select.from_clause()?;
    if matches(&from_clause) {
        return Some(select.clone());
    }
    if from_clause.joins().next().is_some() {
        // Has joins, just none against `enrichment_source` — a genuine
        // structural miss, not a wrapped body.
        return None;
    }
    let mut sources = from_clause.table_refs();
    let only_source = sources.next()?;
    if sources.next().is_some() {
        return None;
    }
    // The two single-source wrap shapes a compiled `body_sql` can arrive
    // already carrying: the SQL compiler's own cast wrap
    // (`smelt_dialect::TYPE_CAST_WRAP_ALIAS`, `_smelt_typed`) and
    // `smelt-runtime`'s `inject_time_filter` output-clamp wrap
    // (`_smelt_output_clamp` — `smelt-logical` cannot depend on
    // `smelt-runtime` to import its constant, so the literal is named here;
    // `docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`
    // discovered the output-clamp shape reaching this function unhandled,
    // silently disabling the declared-`referential_integrity` route's
    // count-preservation probe — and therefore delta restriction — for
    // every live, time-filtered call, both this phase's external-sidecar
    // route and the model-edge route it generalizes). Neither name is
    // load-bearing beyond "some known wrap, not a user's own subquery" —
    // fail closed rather than guess for anything else.
    const OUTPUT_CLAMP_WRAP_ALIAS: &str = "_smelt_output_clamp";
    let alias = only_source.alias();
    if alias.as_deref() != Some(smelt_dialect::TYPE_CAST_WRAP_ALIAS)
        && alias.as_deref() != Some(OUTPUT_CLAMP_WRAP_ALIAS)
    {
        // Not a known wrap shape — some other single-derived-table FROM
        // (a user's own subquery, most commonly). Fail closed rather than
        // guess it's a wrap.
        return None;
    }
    let inner = only_source.subquery()?.select_stmt()?;
    let inner_from = inner.from_clause()?;
    if matches(&inner_from) {
        return Some(inner);
    }
    // Bounded to one level: a wrap nests once, never repeatedly. Do not
    // recurse into `inner` a second time even if it happens to look like
    // another wrap.
    None
}

/// Build [`emit_count_preservation_probe`]'s `driving_select`/`enriched_select`
/// pair directly from a model's own already-compiled `body_sql`, for a
/// caller (`smelt-runtime`'s `execute_delete_insert_with_delta_restriction`)
/// that has the body but not a hand-maintained pair of scoped `SELECT`s: the
/// **enriched** side is `body_sql`'s own top-level `FROM`/`JOIN`/`WHERE`
/// text unchanged (the join against `enrichment_source` still present); the
/// **driving** side is the SAME text with that one join clause spliced out
/// by text range — never re-authored, never re-derived from a second parse
/// of a different string, so both sides carry byte-identical scoping
/// (WHERE, other joins) except for the one join this probe exists to
/// falsify.
///
/// `body_sql` is already-compiled SQL (`smelt.<path>` refs already resolved
/// to physical `schema.table` names by the SQL compiler, matching what
/// `execute_delete_insert_with_delta_restriction` actually holds at its
/// call site) — the join is found by `TableRef::bare_path_text`'s plain
/// dotted-identifier text, never `analysis::source_bounds::
/// resolve_table_ref_source_name` (which only recognises unresolved
/// `smelt.<path>` refs and would never match a compiled body). A match is
/// exact-path or last-segment equality, so a caller may name
/// `enrichment_source` either as the full physical path (`main.dim`) or
/// just its bare table name (`dim`).
///
/// Fail-closed to `None` — never a best-effort guess — when `body_sql` has
/// no top-level `SELECT`, no `FROM` clause, or no join against
/// `enrichment_source` found either in that `FROM` clause's own joins or
/// (see [`select_with_enrichment_join`]) one level inside a single
/// derived-table source — the shape a type-cast wrap produces. A caller
/// that has the model's own pre-wrap body available should still prefer
/// passing that directly; this widening exists so a body that happens to
/// arrive already wrapped is not a spurious miss.
pub fn emit_count_preservation_probe_from_body(
    body_sql: &str,
    enrichment_source: &str,
) -> Option<MaintenanceStatement> {
    let parse = smelt_parser::parse(body_sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let top_select = file.select_stmt()?;
    let select = select_with_enrichment_join(&top_select, enrichment_source)?;
    let from_clause = select.from_clause()?;

    let last_segment = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    let target_last = last_segment(enrichment_source);
    let join = from_clause.joins().find(|join| {
        join.table_ref()
            .and_then(|table_ref| table_ref.bare_path_text())
            .is_some_and(|path| path == enrichment_source || last_segment(&path) == target_last)
    })?;

    let from_range = from_clause.text_range();
    let join_range = join.syntax().text_range();
    let where_suffix = select
        .where_clause()
        .map(|w| format!(" {}", &body_sql[w.text_range()]))
        .unwrap_or_default();

    let enriched_from = &body_sql[from_range];
    let before_join = smelt_parser::TextRange::new(from_range.start(), join_range.start());
    let after_join = smelt_parser::TextRange::new(join_range.end(), from_range.end());
    let driving_from = format!("{}{}", &body_sql[before_join], &body_sql[after_join]);

    let driving_select = format!("SELECT 1 {driving_from}{where_suffix}");
    let enriched_select = format!("SELECT 1 {enriched_from}{where_suffix}");

    Some(emit_count_preservation_probe(
        &driving_select,
        &enriched_select,
    ))
}

/// The functional-dependency probe (`docs/specs/model_properties.md`
/// §"Probe obligation", row `functional_dependencies:`): a read-only query
/// re-aggregating the declared `key` over `scope_select`'s own processed
/// rows and counting the distinct `determines` values found per key. A key
/// with more than one distinct `determines` value disproves the declared
/// per-key constancy the once-write column family was admitted on the
/// strength of (`model_properties.md` §Known Divergences).
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns (`docs/outcomes/20260809-probe-backed-facts/
/// phases/02-plan.md`): the number of offending keys, and up to 5
/// comma-joined offending key values.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed rows; this emitter does no scoping of its own, matching
/// every other function in this module.
///
/// # Panics
/// Panics if `key` is empty — an empty `GROUP BY` would aggregate the whole
/// scope into one row, never a per-key constancy check.
pub fn emit_functional_dependency_probe(
    scope_select: &str,
    key: &[String],
    determines: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !key.is_empty(),
        "emit_functional_dependency_probe requires a non-empty key"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let key_list = key.join(", ");
    let violation_key = probe_key_display_expr(key, cast_type);
    let violations_select = format!(
        "SELECT {violation_key} AS violation_key \
         FROM ({scope_select}) AS __smelt_scope \
         GROUP BY {key_list} \
         HAVING COUNT(DISTINCT {determines}) > 1"
    );
    let sql = wrap_violation_probe("__fd_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// The bounded-domain probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `bounded_domain:`): a read-only query counting the
/// distinct values of the declared `column` within `scope_select`'s own
/// processed region and comparing that count against the declared
/// `max_cardinality`.
///
/// Returns one row: `violation_count` is the distinct-value count when it
/// exceeds `max_cardinality`, `0` when the domain is within cap;
/// `sample_keys` is up to 5 comma-joined sample values, `NULL` when within
/// cap — the same shape every probe in this module returns, specialised
/// (unlike the key-recurring probes) to a magnitude check rather than a
/// membership check.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed region; this emitter does no scoping of its own,
/// matching every other function in this module.
pub fn emit_bounded_domain_probe(
    scope_select: &str,
    column: &str,
    max_cardinality: u64,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let cast_type = probe_dialect_string_type(dialect);
    let sample_expr = probe_dialect_sample_agg(dialect);
    let sql = format!(
        "WITH __bounded_domain_values AS (\
            SELECT DISTINCT CAST({column} AS {cast_type}) AS violation_key \
            FROM ({scope_select}) AS __smelt_scope\
         ), __bounded_domain_count AS (\
            SELECT COUNT(*) AS distinct_count FROM __bounded_domain_values\
         ) \
         SELECT CASE WHEN distinct_count > {max_cardinality} THEN distinct_count ELSE 0 END \
                AS violation_count, \
                CASE WHEN distinct_count > {max_cardinality} THEN \
                  (SELECT {sample_expr} FROM \
                   (SELECT violation_key FROM __bounded_domain_values LIMIT 5) AS __sample) \
                ELSE NULL END AS sample_keys \
         FROM __bounded_domain_count"
    );
    MaintenanceStatement::new(sql)
}

/// The monotonicity probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `timeseries.assert_monotonic`): a read-only query
/// re-deriving the traced event-time ordering over `scope_select`'s own
/// processed rows, per `partition_key` — a `LAG` window over
/// `event_time_column` ordered by itself within each partition, flagging a
/// row whose event time falls below its partition predecessor's.
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns: the number of out-of-order rows, and up to 5
/// comma-joined offending partition-key values.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed rows; this emitter does no scoping of its own, matching
/// every other function in this module.
///
/// # Panics
/// Panics if `partition_key` is empty — an empty partition would order the
/// entire scope as one sequence, never a per-partition monotonicity check.
pub fn emit_monotonicity_probe(
    scope_select: &str,
    partition_key: &[String],
    event_time_column: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !partition_key.is_empty(),
        "emit_monotonicity_probe requires a non-empty partition key"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let partition_list = partition_key.join(", ");
    let violation_key = probe_key_display_expr(partition_key, cast_type);
    // The `LAG` must be ordered by the run's own PROCESSED-row order (a
    // `ROW_NUMBER() OVER ()` ordinal over `scope_select`'s own row
    // sequence), never by `event_time_column` itself — ordering the window
    // by the very column being checked would sort every partition into
    // non-decreasing order by construction, making a violation
    // undetectable. "The traced event-time ordering" (`docs/specs/
    // model_properties.md` §"Probe obligation") means the order rows were
    // processed in, which the caller's `scope_select` already reflects
    // (e.g. an append-only ingestion order).
    let violations_select = format!(
        "SELECT {violation_key} AS violation_key \
         FROM (\
            SELECT {partition_list}, __smelt_event_time, \
                   LAG(__smelt_event_time) OVER (\
                       PARTITION BY {partition_list} ORDER BY __smelt_seq\
                   ) AS __smelt_prev_event_time \
            FROM (\
               SELECT {partition_list}, {event_time_column} AS __smelt_event_time, \
                      ROW_NUMBER() OVER () AS __smelt_seq \
               FROM ({scope_select}) AS __smelt_scope\
            ) AS __smelt_seqd\
         ) AS __smelt_lagged \
         WHERE __smelt_prev_event_time IS NOT NULL \
         AND __smelt_event_time < __smelt_prev_event_time"
    );
    let sql = wrap_violation_probe("__monotonicity_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// One partition's recorded baseline for [`emit_append_only_posture_probe`]:
/// the row count and skeleton-column fingerprint observed the last time the
/// posture was checked, kept by the caller (the run driver, phases 3-4 of
/// `docs/outcomes/20260809-probe-backed-facts/outcome.md`) — this emitter
/// carries no storage of its own, matching the maintenance-plan purity
/// invariant (`docs/specs/architecture.md` §"Constraints & Invariants" item
/// 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOnlyBaselinePartition {
    /// The partition column's value, as text.
    pub partition_value: String,
    /// The row count last recorded for this partition.
    pub recorded_count: i64,
    /// The row-content fingerprint (hex `sha256`) last recorded for this
    /// partition, over the same `digest_columns` the caller passes to
    /// [`emit_append_only_posture_probe`].
    pub recorded_fingerprint: String,
    /// Whether this partition's fingerprint leg is checked this run. A
    /// whole-partition fingerprint changes on a *legitimate* append, so the
    /// caller gates this to `false` for the still-open partition (the
    /// recorded maximum `partition_value`) and `true` for every partition
    /// strictly below it (`docs/outcomes/20260809-probe-backed-facts/
    /// outcome.md` phase 6: "the frontier gate"). The count leg is never
    /// gated — a count decrease always violates append-only posture
    /// regardless of this flag.
    pub check_fingerprint: bool,
}

/// The append-only posture probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `mutation_profile.kind: append_only`; `docs/specs/
/// sources.md` §Semantics 4 — "watermark-monotonicity + frontier-checksum"):
/// a read-only query comparing each partition's CURRENT row count and
/// row-content fingerprint (over `digest_columns`, the source's skeleton
/// columns) against a caller-supplied recorded `baseline`. This is the
/// *violation* half of the two-verdict posture split
/// (`docs/specs/model_properties.md` §Constraints "Declared lateness is
/// orchestration-only"): a partition whose row count decreased (a delete or
/// reload), or whose fingerprint changed **at an unchanged row count** (an
/// in-place update), disproves the declared `append_only` posture. A row-count
/// *increase* in a closed partition is deliberately NOT a violation here —
/// that is a late append, classified separately by the pure [`late_appends`]
/// over the same baseline and the caller-executed
/// [`emit_append_only_baseline_snapshot`] current state.
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns: the number of violating partitions, and up to 5
/// comma-joined offending partition values.
///
/// The per-row fingerprint reuses [`row_fingerprint_expr`] (the same
/// collision-free construction the fingerprint sidecar digests with); the
/// per-partition aggregate fingerprint is `sha256` of those row
/// fingerprints joined in a fixed (sorted) order, so two runs over the same
/// unchanged partition content always agree regardless of physical row
/// order.
///
/// `source_table` is already fully qualified (`schema.table`); `baseline`
/// is rendered as a `VALUES` list, one row per recorded partition — a
/// partition with no baseline row is not compared (nothing recorded yet to
/// disprove). Each baseline row's `check_fingerprint` gates whether that
/// partition's fingerprint leg participates this run — the count leg is
/// never gated (`AppendOnlyBaselinePartition::check_fingerprint`'s doc
/// comment).
///
/// # Panics
/// Panics if `digest_columns` or `baseline` is empty — nothing to
/// fingerprint, or nothing recorded to compare against, is not a
/// degenerate always-passing probe.
pub fn emit_append_only_posture_probe(
    source_table: &str,
    partition_column: &str,
    digest_columns: &[String],
    baseline: &[AppendOnlyBaselinePartition],
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !digest_columns.is_empty(),
        "emit_append_only_posture_probe requires a non-empty digest column set for {source_table}"
    );
    assert!(
        !baseline.is_empty(),
        "emit_append_only_posture_probe requires a non-empty recorded baseline for {source_table}"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let snapshot =
        emit_append_only_baseline_snapshot(source_table, partition_column, digest_columns, dialect);
    let baseline_rows: Vec<Vec<String>> = baseline
        .iter()
        .map(|b| {
            let value = b.partition_value.replace('\'', "''");
            let fingerprint = b.recorded_fingerprint.replace('\'', "''");
            let check_fingerprint = if b.check_fingerprint { "TRUE" } else { "FALSE" };
            vec![
                format!("'{value}'"),
                b.recorded_count.to_string(),
                format!("'{fingerprint}'"),
                check_fingerprint.to_string(),
            ]
        })
        .collect();
    // The row-set constructor itself — `VALUES (…)` where `dialect`
    // supports one, the portable `SELECT … UNION ALL SELECT …` rewrite
    // GoogleSQL requires otherwise — comes from
    // `smelt_core::build_row_set_table`, the single dialect-aware owner.
    let baseline_row_set = smelt_core::build_row_set_table(
        maintenance_dialect_to_backend_type(dialect),
        "__baseline",
        &[
            "partition_value",
            "recorded_count",
            "recorded_fingerprint",
            "check_fingerprint",
        ],
        &baseline_rows,
    );
    let violations_select = format!(
        "SELECT CAST(__current.partition_value AS {cast_type}) AS violation_key \
         FROM ({}) AS __current \
         JOIN {baseline_row_set} \
         ON __current.partition_value = __baseline.partition_value \
         WHERE __current.current_count < __baseline.recorded_count \
         OR (__baseline.check_fingerprint AND __current.current_count = __baseline.recorded_count \
             AND __current.current_fingerprint IS DISTINCT FROM __baseline.recorded_fingerprint)",
        snapshot.sql
    );
    let sql = wrap_violation_probe("__append_only_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// One partition's current per-partition state, the shape
/// [`late_appends`] compares against a recorded
/// [`AppendOnlyBaselinePartition`] — the row shape
/// [`emit_append_only_baseline_snapshot`]'s `SELECT` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentPartitionState {
    /// The partition column's value, as text.
    pub partition_value: String,
    /// The row count observed for this partition right now.
    pub current_count: i64,
}

/// A genuine late append: a closed partition (`check_fingerprint: true` in
/// the recorded baseline) whose row count increased since that baseline was
/// recorded — an observed delta the next run re-processes, never a posture
/// violation (`docs/specs/model_properties.md` §Constraints "Declared
/// lateness is orchestration-only").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LateAppend {
    pub partition_value: String,
    /// The row-count increase since the recorded baseline.
    pub added_rows: i64,
}

/// Classifies `current` against `baseline`, mirroring
/// [`crate::contract::frozen_horizon::late_arrivals`]'s shape: a partition
/// absent from `baseline` is an ordinary append (nothing recorded yet to
/// disprove), the still-open frontier partition (`check_fingerprint:
/// false`) is never reported (it legitimately gains rows every run), and
/// among the remaining closed partitions only a row-count **increase**
/// counts — a decrease or an unchanged count (including one with a changed
/// fingerprint, the in-place-update case) is not a late append; those are
/// [`emit_append_only_posture_probe`]'s violation predicate's concern
/// instead, never both. A delete-plus-insert that nets to a count increase
/// is therefore classified as a late append: one aggregate fingerprint per
/// partition cannot prove subset-ness, so the count leg governs. Pure — no
/// I/O; the caller supplies `current` from
/// [`emit_append_only_baseline_snapshot`]'s executed row set.
pub fn late_appends(
    baseline: &[AppendOnlyBaselinePartition],
    current: &[CurrentPartitionState],
) -> Vec<LateAppend> {
    use std::collections::HashMap;

    let recorded: HashMap<&str, &AppendOnlyBaselinePartition> = baseline
        .iter()
        .map(|b| (b.partition_value.as_str(), b))
        .collect();

    let mut late: Vec<LateAppend> = current
        .iter()
        .filter_map(|c| {
            let b = recorded.get(c.partition_value.as_str())?;
            if !b.check_fingerprint {
                return None;
            }
            if c.current_count > b.recorded_count {
                Some(LateAppend {
                    partition_value: c.partition_value.clone(),
                    added_rows: c.current_count - b.recorded_count,
                })
            } else {
                None
            }
        })
        .collect();
    late.sort_by(|a, b| a.partition_value.cmp(&b.partition_value));
    late
}

/// The per-partition CURRENT-state `SELECT` [`emit_append_only_posture_probe`]
/// compares its recorded baseline against: `partition_value`,
/// `current_count`, and `current_fingerprint` for every partition presently
/// in `source_table`. Extracted as its own emitter so the runtime can
/// execute it standalone to refresh the recorded baseline after a held
/// probe (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 6) —
/// the recorded and compared fingerprints are then the literally same SQL
/// construction, never two independent renderings that could drift.
///
/// `source_table` is already fully qualified (`schema.table`). The
/// fingerprint construction is identical to
/// [`emit_append_only_posture_probe`]'s own (see that function's doc
/// comment).
///
/// # Panics
/// Panics if `digest_columns` is empty — nothing to fingerprint is not a
/// degenerate probe.
pub fn emit_append_only_baseline_snapshot(
    source_table: &str,
    partition_column: &str,
    digest_columns: &[String],
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !digest_columns.is_empty(),
        "emit_append_only_baseline_snapshot requires a non-empty digest column set for {source_table}"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let row_hash = row_fingerprint_expr(digest_columns, dialect);
    let agg_fingerprint = match dialect {
        MaintenanceDialect::DuckDb => {
            format!("sha256(STRING_AGG({row_hash}, '' ORDER BY {row_hash}))")
        }
        MaintenanceDialect::Spark => {
            format!("sha256(CONCAT_WS('', SORT_ARRAY(COLLECT_LIST({row_hash}))))")
        }
        // GoogleSQL's SHA256 returns BYTES rather than a hex string, so the
        // digest is wrapped in TO_HEX to keep the fingerprint a STRING the way
        // every other dialect's is. Confirmed live (scripts/bigquery-probe3.sh).
        MaintenanceDialect::BigQuery => {
            format!("TO_HEX(SHA256(STRING_AGG({row_hash}, '' ORDER BY {row_hash})))")
        }
    };
    let sql = format!(
        "SELECT CAST({partition_column} AS {cast_type}) AS partition_value, \
         COUNT(*) AS current_count, {agg_fingerprint} AS current_fingerprint \
         FROM {source_table} \
         GROUP BY {partition_column}"
    );
    MaintenanceStatement::new(sql)
}

/// The whole-source (unpartitioned) counterpart of
/// [`emit_append_only_baseline_snapshot`]: a source's CURRENT row count and
/// order-independent content fingerprint over `digest_columns`, with no
/// `GROUP BY` and no partition column — the mutation-happened discrimination
/// baseline an `UpstreamMutation` cell's dispatch decision compares against
/// (`docs/specs/incremental_models.md` §"When a mutation cell dispatches").
///
/// The fingerprint construction is identical to
/// [`emit_append_only_baseline_snapshot`]'s own (same per-row hash via
/// [`row_fingerprint_expr`], same sorted-`STRING_AGG`-then-`sha256`
/// aggregate, same per-dialect forms) — this emitter differs only in scope
/// (the whole source, not one partition at a time).
///
/// `source_table` is already fully qualified (`schema.table`).
///
/// # Panics
/// Panics if `digest_columns` is empty — nothing to fingerprint is not a
/// degenerate probe.
pub fn emit_source_mutation_fingerprint(
    source_table: &str,
    digest_columns: &[String],
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !digest_columns.is_empty(),
        "emit_source_mutation_fingerprint requires a non-empty digest column set for \
         {source_table}"
    );
    let row_hash = row_fingerprint_expr(digest_columns, dialect);
    let agg_fingerprint = match dialect {
        MaintenanceDialect::DuckDb => {
            format!("sha256(STRING_AGG({row_hash}, '' ORDER BY {row_hash}))")
        }
        MaintenanceDialect::Spark => {
            format!("sha256(CONCAT_WS('', SORT_ARRAY(COLLECT_LIST({row_hash}))))")
        }
        MaintenanceDialect::BigQuery => {
            format!("TO_HEX(SHA256(STRING_AGG({row_hash}, '' ORDER BY {row_hash})))")
        }
    };
    let sql = format!(
        "SELECT COUNT(*) AS current_count, {agg_fingerprint} AS current_fingerprint \
         FROM {source_table}"
    );
    MaintenanceStatement::new(sql)
}

#[cfg(test)]
mod count_preservation_probe_tests {
    use super::*;

    #[test]
    fn probe_compares_driving_and_enriched_row_counts() {
        let probe = emit_count_preservation_probe(
            "SELECT event_id FROM main.raw_events WHERE event_date >= '2026-07-01'",
            "SELECT e.event_id FROM main.raw_events e JOIN main.raw_users u ON e.user_id = \
             u.user_id WHERE e.event_date >= '2026-07-01'",
        );
        assert_eq!(
            probe.sql,
            "SELECT (SELECT COUNT(*) FROM (SELECT event_id FROM main.raw_events WHERE \
             event_date >= '2026-07-01') AS __smelt_driving) AS driving_count, (SELECT \
             COUNT(*) FROM (SELECT e.event_id FROM main.raw_events e JOIN main.raw_users u ON \
             e.user_id = u.user_id WHERE e.event_date >= '2026-07-01') AS __smelt_enriched) AS \
             enriched_count"
        );
    }
}
