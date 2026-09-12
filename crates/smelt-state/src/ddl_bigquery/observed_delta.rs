//! Warehouse-resident observed-output-delta record, GoogleSQL spelling.
//!
//! The DuckDB builders in `crate::ddl_duckdb` own the record's *meaning*
//! (`docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas
//! on model edges"); this module owns nothing but its GoogleSQL spelling. Same
//! table name, same five columns, same three-column key, so a project moved
//! from one dialect to the other reads the same shape back.
//!
//! Five GoogleSQL facts make the DuckDB text wrong here rather than merely
//! differently spelled, and each is a fact of the realisation recorded in
//! `docs/specs/state.md` §"Which dialects realise which structure":
//!
//! - **There is no `ON CONFLICT … DO UPDATE`.** The idempotent-replace upsert
//!   is re-expressed as `MERGE … WHEN MATCHED THEN UPDATE … WHEN NOT MATCHED
//!   THEN INSERT`, the same shape the ledger's upsert uses, matched on the
//!   same three columns the `PRIMARY KEY` names.
//! - **There is no `FILTER (WHERE …)` clause.** GoogleSQL's own form is
//!   `ARRAY_AGG(DISTINCT x IGNORE NULLS)`, and it is load-bearing rather than
//!   cosmetic: `ARRAY_AGG` **raises** on a NULL element in GoogleSQL (an array
//!   cannot hold a NULL), so dropping `IGNORE NULLS` would turn a window with
//!   one unmatched key into a failed run rather than a narrower delta.
//! - **`ARRAY_AGG` over zero input rows is `NULL`**, exactly as in DuckDB, so
//!   the `COALESCE` to an empty array is kept — it is what makes a
//!   fully-suppressed run record a *present-and-empty* row rather than one
//!   carrying a NULL. The empty literal is spelled `ARRAY<STRING>[]`: a bare
//!   `[]` has no element type for `COALESCE` to unify against the aggregate's.
//! - **Array element types are not coerced on write.** `delta_key` and
//!   `delta_partition` come from the caller's own query over user columns and
//!   may be any type — `delta_partition` is literally `NULL AS
//!   delta_partition` (an INT64-typed NULL) for a model with no partition
//!   axis. DuckDB casts an `INTEGER[]` into a `VARCHAR[]` column implicitly;
//!   GoogleSQL does not, so each element is `CAST(… AS STRING)` inside the
//!   aggregate. Without it a first non-STRING key type is a type error at the
//!   `MERGE`, and the `DISTINCT` would be over the wrong domain.
//! - **The array columns are declared without `NOT NULL`.** BigQuery cannot
//!   represent a NULL array at all — a NULL written to an `ARRAY` column reads
//!   back as empty — so the constraint would be either redundant or refused,
//!   and neither is worth a live-only failure mode. Nothing depends on it:
//!   see [`generate_observed_delta_select_sql`] for why the empty-vs-absent
//!   distinction this table exists to carry is **row presence**, never a NULL.
//!
//! Kept in `smelt-state` beside its DuckDB twin under the same bookkeeping
//! exclusion from the maintenance-plan-purity invariant (`CLAUDE.md`
//! §"Maintenance-plan purity").

use super::{escape_string_literal, qualified};

/// The observed-delta table's five columns, in storage order. One list, used
/// by the DDL and by both halves of the `MERGE`, so they cannot drift.
const OBSERVED_DELTA_COLUMNS: [&str; 5] = [
    "model_name",
    "window_start",
    "window_end",
    "changed_keys",
    "partitions",
];

/// The three columns that identify one observed-delta row — the `PRIMARY KEY`,
/// and the `MERGE`'s match condition.
const OBSERVED_DELTA_KEY_COLUMNS: [&str; 3] = ["model_name", "window_start", "window_end"];

fn observed_delta_table(schema: &str) -> String {
    qualified(schema, crate::ddl_duckdb::OBSERVED_DELTA_TABLE_NAME)
}

/// GoogleSQL DDL creating the observed-delta table if it does not already
/// exist. Idempotent — safe to run before every conditional write. GoogleSQL
/// counterpart of [`crate::ddl_duckdb::generate_observed_delta_table_ddl`].
///
/// The `PRIMARY KEY` is declared `NOT ENFORCED` because GoogleSQL has no other
/// form; it documents row identity and refuses nothing. Nothing here relies on
/// it — the idempotent replace is the `MERGE`'s own match condition, not a
/// conflict the storage engine detects.
pub fn generate_observed_delta_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\
         model_name STRING NOT NULL, \
         window_start STRING NOT NULL, \
         window_end STRING NOT NULL, \
         changed_keys ARRAY<STRING>, \
         partitions ARRAY<STRING>, \
         PRIMARY KEY ({}) NOT ENFORCED)",
        observed_delta_table(schema),
        OBSERVED_DELTA_KEY_COLUMNS.join(", "),
    )
}

/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_observed_delta_upsert_sql`]: upsert one
/// `(model, run window)`'s observed delta, replacing any row already recorded
/// for that window.
///
/// `changed_keys_query` is the caller's already-built SELECT of every changed
/// row's key value(s) as a `delta_key` column and its touched-partition
/// projection as a `delta_partition` column — identical to the DuckDB
/// contract, and built by the same single-owner emitters, so this module
/// neither knows nor cares what is inside it.
///
/// The statement **always writes exactly one row** for the window: the source
/// is one un-grouped aggregate `SELECT`, which yields one row even over zero
/// input rows, and the `COALESCE` turns that row's `NULL` aggregates into
/// empty arrays. That is what makes a fully-suppressed run *present and empty*
/// rather than absent (`docs/specs/incremental_models.md` §"The graph layer" —
/// "Empty and absent are distinct").
pub fn generate_observed_delta_upsert_sql(
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
    changed_keys_query: &str,
) -> String {
    let on_clause = OBSERVED_DELTA_KEY_COLUMNS
        .iter()
        .map(|c| format!("T.{c} = S.{c}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let insert_values = OBSERVED_DELTA_COLUMNS
        .iter()
        .map(|c| format!("S.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "MERGE {table} T USING (\
         SELECT '{model}' AS model_name, '{window_start}' AS window_start, \
         '{window_end}' AS window_end, \
         COALESCE(ARRAY_AGG(DISTINCT CAST(__smelt_delta.delta_key AS STRING) IGNORE NULLS), \
         ARRAY<STRING>[]) AS changed_keys, \
         COALESCE(ARRAY_AGG(DISTINCT CAST(__smelt_delta.delta_partition AS STRING) IGNORE NULLS), \
         ARRAY<STRING>[]) AS partitions \
         FROM ({changed_keys_query}) AS __smelt_delta\
         ) S ON {on_clause} \
         WHEN MATCHED THEN UPDATE SET changed_keys = S.changed_keys, partitions = S.partitions \
         WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ({insert_values})",
        table = observed_delta_table(schema),
        model = escape_string_literal(model),
        window_start = escape_string_literal(window_start),
        window_end = escape_string_literal(window_end),
        changed_keys_query = changed_keys_query,
        on_clause = on_clause,
        columns = OBSERVED_DELTA_COLUMNS.join(", "),
        insert_values = insert_values,
    )
}

/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_observed_delta_select_sql`]: read one
/// `(model, window)`'s recorded row back.
///
/// **Why BigQuery's array flattening cannot break "empty and absent are
/// distinct".** BigQuery does not distinguish a NULL `ARRAY` from an empty
/// one: a NULL array written to a column reads back as empty. That would be
/// fatal if *absent* meant "a NULL array column" — it does not. Absent means
/// **no row for the window**, and the caller's own row count is what reports
/// it (`smelt_runtime::maintenance_driver::read_observed_delta` returns `None`
/// for zero rows, `Some` — possibly with both vectors empty — otherwise).
/// Since [`generate_observed_delta_upsert_sql`] always writes exactly one row
/// per recorded window, and this `SELECT` filters only on the window key, the
/// distinction the system depends on is carried entirely by row presence,
/// which no array-typed flattening can touch.
pub fn generate_observed_delta_select_sql(
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> String {
    format!(
        "SELECT changed_keys, partitions FROM {table} \
         WHERE model_name = '{model}' AND window_start = '{window_start}' \
         AND window_end = '{window_end}'",
        table = observed_delta_table(schema),
        model = escape_string_literal(model),
        window_start = escape_string_literal(window_start),
        window_end = escape_string_literal(window_end),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY: &str = "SELECT user_id AS delta_key, NULL AS delta_partition FROM main.t";

    #[test]
    fn table_ddl_is_googlesql_not_duckdb() {
        let ddl = generate_observed_delta_table_ddl("ds");
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS `ds._smelt_observed_delta` ("));
        assert!(ddl.contains("model_name STRING NOT NULL"));
        assert!(ddl.contains("changed_keys ARRAY<STRING>,"));
        assert!(ddl.contains("partitions ARRAY<STRING>,"));
        assert!(
            ddl.contains("PRIMARY KEY (model_name, window_start, window_end) NOT ENFORCED)"),
            "{ddl}"
        );
        assert!(!ddl.contains("VARCHAR"), "{ddl}");
        assert!(!ddl.contains('"'), "{ddl}");
        // The array columns carry no NOT NULL: BigQuery cannot store a NULL
        // array, and the empty-vs-absent distinction is row presence.
        assert!(!ddl.contains("ARRAY<STRING> NOT NULL"), "{ddl}");
    }

    #[test]
    fn upsert_is_a_merge_with_ignore_nulls_and_no_duckdb_isms() {
        let sql = generate_observed_delta_upsert_sql("ds", "dim_scores", "s", "e", QUERY);
        assert!(
            sql.starts_with("MERGE `ds._smelt_observed_delta` T USING ("),
            "{sql}"
        );
        assert!(sql.contains("WHEN MATCHED THEN UPDATE SET changed_keys = S.changed_keys, partitions = S.partitions"), "{sql}");
        assert!(
            sql.contains(
                "WHEN NOT MATCHED THEN INSERT (model_name, window_start, window_end, \
                 changed_keys, partitions) VALUES (S.model_name, S.window_start, \
                 S.window_end, S.changed_keys, S.partitions)"
            ),
            "{sql}"
        );
        assert!(
            sql.contains(
                "ON T.model_name = S.model_name AND T.window_start = S.window_start \
                          AND T.window_end = S.window_end"
            ),
            "{sql}"
        );
        assert!(!sql.contains("ON CONFLICT"), "{sql}");
        assert!(!sql.contains("FILTER (WHERE"), "{sql}");
        assert!(!sql.contains("::VARCHAR[]"), "{sql}");
        assert!(!sql.contains("excluded."), "{sql}");
        assert!(!sql.contains('"'), "{sql}");
    }

    /// `IGNORE NULLS` and the `CAST` are both correctness, not style: without
    /// the first, a NULL element makes GoogleSQL's `ARRAY_AGG` raise; without
    /// the second, a non-STRING `delta_key`/`delta_partition` (the latter is
    /// literally `NULL AS delta_partition`, an INT64) is a type error against
    /// the `ARRAY<STRING>` column.
    #[test]
    fn both_aggregates_ignore_nulls_and_cast_to_string() {
        let sql = generate_observed_delta_upsert_sql("ds", "m", "s", "e", QUERY);
        assert!(
            sql.contains(
                "ARRAY_AGG(DISTINCT CAST(__smelt_delta.delta_key AS STRING) IGNORE NULLS)"
            ),
            "{sql}"
        );
        assert!(
            sql.contains(
                "ARRAY_AGG(DISTINCT CAST(__smelt_delta.delta_partition AS STRING) IGNORE NULLS)"
            ),
            "{sql}"
        );
    }

    /// A fully-suppressed run must still record a row, and it must be
    /// present-and-empty rather than NULL-bearing: the source is one
    /// un-grouped aggregate `SELECT` (always exactly one row, even over zero
    /// input rows) and both aggregates are wrapped in a `COALESCE` to the
    /// empty typed array.
    #[test]
    fn a_suppressed_run_still_records_a_present_and_empty_row() {
        let sql = generate_observed_delta_upsert_sql("ds", "m", "s", "e", QUERY);
        assert_eq!(
            sql.matches("ARRAY<STRING>[])").count(),
            2,
            "both aggregates must COALESCE to the empty typed array: {sql}"
        );
        assert!(
            !sql.contains("GROUP BY"),
            "the source must stay un-grouped so it yields one row over zero input rows: {sql}"
        );
        // A bare `[]` has no element type to unify with the aggregate's.
        assert!(
            !sql.contains("COALESCE(ARRAY_AGG(DISTINCT __smelt_delta"),
            "{sql}"
        );
    }

    /// Absence is **row absence**: the read filters on the window key alone
    /// and projects the two array columns, so BigQuery's inability to
    /// distinguish a NULL array from an empty one cannot reach the
    /// distinction.
    #[test]
    fn select_distinguishes_windows_by_row_presence_only() {
        let sql = generate_observed_delta_select_sql("ds", "dim_scores", "s", "e");
        assert_eq!(
            sql,
            "SELECT changed_keys, partitions FROM `ds._smelt_observed_delta` \
             WHERE model_name = 'dim_scores' AND window_start = 's' AND window_end = 'e'"
        );
        assert!(!sql.contains("IS NOT NULL"), "{sql}");
    }

    #[test]
    fn string_literals_use_googlesql_backslash_escaping() {
        let sql = generate_observed_delta_select_sql("ds", "it's\\here", "s", "e");
        assert!(sql.contains(r"'it\'s\\here'"), "{sql}");
        assert!(!sql.contains("''"), "{sql}");
    }

    /// A schema that already carries a project prefix stays one backticked
    /// path, matching `smelt_backend_bigquery::sql::qualified_name`.
    #[test]
    fn a_project_prefixed_schema_stays_one_path() {
        assert!(generate_observed_delta_table_ddl("my-proj.ds")
            .contains("`my-proj.ds._smelt_observed_delta`"));
    }
}
