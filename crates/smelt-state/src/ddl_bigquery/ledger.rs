//! Warehouse-resident ledger, GoogleSQL spelling.
//!
//! Split out of `ddl_bigquery/mod.rs` (which owns the schema-evolution DDL)
//! so neither half grows into a token-cost hot spot; `super`'s `qualified` and
//! `escape_string_literal` are shared with the observed-delta sibling.

use super::{escape_string_literal, qualified};

//
// The DuckDB builders in `crate::ddl_duckdb` own the ledger's *meaning*
// (`docs/specs/incremental_models.md` §"The frontier record (reconciliation
// ledger)", `docs/specs/incremental_shapes.md` §"The transactional frontier
// write (merge ledger)"); this section owns nothing but its GoogleSQL
// spelling. Same table name, same six columns, same key — a project moved
// from one dialect to the other reads the same ledger shape.
//
// Three GoogleSQL facts make the DuckDB text a hard error rather than a
// dialect wobble, and each is stated as a fact of the realisation in
// `docs/specs/state.md` §"Which dialects realise which structure":
//
// - **Identifiers.** `"schema"` is a *string literal* in GoogleSQL, not an
//   identifier. The whole path is backticked once (`qualified`), the shape
//   `smelt_backend_bigquery::sql::qualified_name` produces, which keeps a
//   schema that already carries a project prefix working. The driver passes
//   only `schema`, so the emitted name is two-part and resolves against the
//   query job's own default project.
// - **`PRIMARY KEY` must say `NOT ENFORCED`** — a bare `PRIMARY KEY` is a
//   syntax error. The declaration is documentation and an optimiser hint;
//   BigQuery never enforces it. Nothing here may rely on the key to refuse a
//   duplicate, which is precisely why the *additive* never-fold-twice refusal
//   — on DuckDB, that constraint violation itself — is not realisable on
//   BigQuery from this module alone.
// - **There is no `ON CONFLICT DO NOTHING`.** The re-run-tolerant upsert is
//   re-expressed as `MERGE … WHEN NOT MATCHED THEN INSERT` against a one-row
//   inline source. `SELECT <literals>` is the source spelling rather than
//   `UNNEST([STRUCT(…)])`: the scaling form matters for a row *set* (a
//   chained `SELECT … UNION ALL …` costs per-row planning, which is why
//   observed-delta row sets use `UNNEST`), and this source is always exactly
//   one row, so the simpler spelling has no cost to avoid.
//
// Kept in `smelt-state` beside its DuckDB twin under the same bookkeeping
// exclusion from the maintenance-plan-purity invariant (`CLAUDE.md`
// §"Maintenance-plan purity" — "ledger DDL/DML in `smelt-state` excluded as
// bookkeeping").

/// The ledger's six columns, in storage order. One list, used by the DDL, the
/// `INSERT` and the `MERGE`, so the three cannot drift out of agreement.
const LEDGER_COLUMNS: [&str; 6] = [
    "model_name",
    "grp",
    "input_name",
    "delta_id",
    "region_start",
    "region_end",
];

/// The four columns that identify one ledger row — the `PRIMARY KEY`, and the
/// `MERGE`'s match condition.
const LEDGER_KEY_COLUMNS: [&str; 4] = ["model_name", "grp", "input_name", "delta_id"];

fn ledger_table(schema: &str) -> String {
    qualified(schema, crate::ddl_duckdb::LEDGER_TABLE_NAME)
}

/// GoogleSQL DDL creating the ledger table if it does not already exist.
/// Idempotent — safe to run before every fold. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_table_ddl`].
///
/// The `PRIMARY KEY` is declared `NOT ENFORCED` because GoogleSQL has no other
/// form: it documents the row identity and informs the optimiser, and refuses
/// nothing at write time.
pub fn generate_ledger_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\
         model_name STRING NOT NULL, \
         grp STRING NOT NULL, \
         input_name STRING NOT NULL, \
         delta_id STRING NOT NULL, \
         region_start STRING NOT NULL, \
         region_end STRING NOT NULL, \
         PRIMARY KEY ({}) NOT ENFORCED)",
        ledger_table(schema),
        LEDGER_KEY_COLUMNS.join(", "),
    )
}

/// GoogleSQL `INSERT` recording one delta identity as folded for `(model,
/// group, input)`. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_insert_sql`] — with the one semantic
/// difference that matters: it does **not** refuse a repeat, because the
/// table's `PRIMARY KEY` is unenforced. A caller needing the never-fold-twice
/// refusal must obtain it some other way on this dialect.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_insert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    format!(
        "INSERT INTO {} ({}) VALUES ('{}', '{}', '{}', '{}', '{}', '{}')",
        ledger_table(schema),
        LEDGER_COLUMNS.join(", "),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(input),
        escape_string_literal(delta_id),
        escape_string_literal(region_start),
        escape_string_literal(region_end),
    )
}

/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_upsert_sql`]'s `ON CONFLICT DO
/// NOTHING` — the **bookkeeping** record of a re-run-tolerant
/// (`Grade::Idempotent`) window-forward keyed model's merged window.
///
/// GoogleSQL has no conflict clause, so the no-op-on-repeat behaviour is
/// expressed directly: `MERGE … WHEN NOT MATCHED THEN INSERT` against a
/// one-row inline source, matched on the same four key columns the DuckDB
/// table's `PRIMARY KEY` names. Re-recording an already-recorded window
/// touches nothing — the same observable behaviour, and it does not depend on
/// the unenforced key.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_upsert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    let values = [model, group, input, delta_id, region_start, region_end];
    let source_select = LEDGER_COLUMNS
        .iter()
        .zip(values.iter())
        .map(|(column, value)| format!("'{}' AS {}", escape_string_literal(value), column))
        .collect::<Vec<_>>()
        .join(", ");
    let on_clause = LEDGER_KEY_COLUMNS
        .iter()
        .map(|c| format!("T.{c} = S.{c}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let insert_values = LEDGER_COLUMNS
        .iter()
        .map(|c| format!("S.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "MERGE {} T USING (SELECT {}) S ON {} WHEN NOT MATCHED THEN INSERT ({}) VALUES ({})",
        ledger_table(schema),
        source_select,
        on_clause,
        LEDGER_COLUMNS.join(", "),
        insert_values,
    )
}

/// The GoogleSQL realisation of the **never-fold-twice record**: insert this
/// delta identity, or touch nothing if it is already there
/// (`docs/specs/incremental_models.md` §Constraints "Never fold a delta
/// already reflected in the state").
///
/// DuckDB realises the same guarantee with
/// [`crate::ddl_duckdb::generate_ledger_insert_sql`] and an *enforced*
/// `PRIMARY KEY`: a repeat raises a constraint violation, and the violation is
/// the refusal. GoogleSQL's `PRIMARY KEY` is `NOT ENFORCED` and raises nothing,
/// so the refusal has to come from the statement's own effect instead — this
/// one inserts zero rows on a repeat, and the caller
/// (`smelt_backend_bigquery::sql::fold_ledger_delta_script`) turns a zero-row
/// outcome into the abort, inside the transaction that also holds the fold.
///
/// The statement itself is [`generate_ledger_upsert_sql`]'s `MERGE … WHEN NOT
/// MATCHED THEN INSERT` verbatim — the two roles differ only in how the caller
/// reads the row count, never in the text, so they are deliberately one
/// builder rather than two spellings that could drift. Delegating also means
/// the additive fold and the idempotent bookkeeping record can never disagree
/// about what "this window is recorded" means.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_conditional_insert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    generate_ledger_upsert_sql(
        schema,
        model,
        group,
        input,
        delta_id,
        region_start,
        region_end,
    )
}

/// GoogleSQL existence check for `(model, group, input, delta_id)` — the
/// best-effort fallback `Backend::fold_ledger_delta` default uses on a backend
/// that cannot wrap the insert and the fold action in one native transaction.
/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_exists_sql`].
pub fn generate_ledger_exists_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
) -> String {
    format!(
        "SELECT 1 FROM {} WHERE model_name = '{}' AND grp = '{}' AND input_name = '{}' \
         AND delta_id = '{}' LIMIT 1",
        ledger_table(schema),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(input),
        escape_string_literal(delta_id),
    )
}

/// GoogleSQL `DELETE` + `INSERT` implementing the ledger's region-recompute
/// reset. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_recompute_reset_sqls`], with the same
/// half-open intersection test and the same ordering contract (run both, in
/// this order, inside one backend transaction alongside the recompute's own
/// write).
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_recompute_reset_sqls(
    schema: &str,
    model: &str,
    group: &str,
    region_start: &str,
    region_end: &str,
    input: &str,
    delta_id: &str,
) -> Vec<String> {
    let delete_sql = format!(
        "DELETE FROM {} WHERE model_name = '{}' AND grp = '{}' \
         AND region_start < '{}' AND region_end > '{}'",
        ledger_table(schema),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(region_end),
        escape_string_literal(region_start),
    );
    let insert_sql = generate_ledger_insert_sql(
        schema,
        model,
        group,
        input,
        delta_id,
        region_start,
        region_end,
    );
    vec![delete_sql, insert_sql]
}

#[cfg(test)]
mod ledger_tests {
    use super::*;

    #[test]
    fn the_ddl_declares_a_not_enforced_primary_key_over_a_backticked_two_part_name() {
        assert_eq!(
            generate_ledger_table_ddl("smelt_dogfood"),
            "CREATE TABLE IF NOT EXISTS `smelt_dogfood._smelt_ledger` (\
             model_name STRING NOT NULL, grp STRING NOT NULL, input_name STRING NOT NULL, \
             delta_id STRING NOT NULL, region_start STRING NOT NULL, region_end STRING NOT NULL, \
             PRIMARY KEY (model_name, grp, input_name, delta_id) NOT ENFORCED)"
        );
    }

    #[test]
    fn the_ddl_carries_no_duckdb_type_name_and_no_double_quoted_identifier() {
        let ddl = generate_ledger_table_ddl("ds");
        assert!(!ddl.contains("VARCHAR"), "{ddl}");
        assert!(!ddl.contains('"'), "{ddl}");
    }

    #[test]
    fn the_insert_lists_its_columns_and_backticks_the_table() {
        assert_eq!(
            generate_ledger_insert_sql(
                "ds",
                "silver.events_deduped",
                "{*}",
                "smelt.raw_events",
                "2026-01-01",
                "2026-01-01",
                "2026-01-02",
            ),
            "INSERT INTO `ds._smelt_ledger` \
             (model_name, grp, input_name, delta_id, region_start, region_end) \
             VALUES ('silver.events_deduped', '{*}', 'smelt.raw_events', '2026-01-01', \
             '2026-01-01', '2026-01-02')"
        );
    }

    /// The row's named defect: GoogleSQL has no `ON CONFLICT DO NOTHING`.
    #[test]
    fn the_upsert_is_a_merge_when_not_matched_and_never_on_conflict() {
        let sql = generate_ledger_upsert_sql(
            "ds",
            "m",
            "{*}",
            "smelt.src",
            "2026-01-01",
            "2026-01-01",
            "2026-01-02",
        );
        assert_eq!(
            sql,
            "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS model_name, '{*}' AS grp, \
             'smelt.src' AS input_name, '2026-01-01' AS delta_id, '2026-01-01' AS region_start, \
             '2026-01-02' AS region_end) S ON T.model_name = S.model_name AND T.grp = S.grp AND \
             T.input_name = S.input_name AND T.delta_id = S.delta_id WHEN NOT MATCHED THEN \
             INSERT (model_name, grp, input_name, delta_id, region_start, region_end) \
             VALUES (S.model_name, S.grp, S.input_name, S.delta_id, S.region_start, S.region_end)"
        );
        assert!(!sql.contains("ON CONFLICT"), "{sql}");
    }

    /// The source is one row, so it needs no `UNNEST([STRUCT(…)])` — and it
    /// must not be a chained `UNION ALL` either.
    #[test]
    fn the_upsert_source_is_a_single_select_row() {
        let sql = generate_ledger_upsert_sql("ds", "m", "g", "i", "d", "s", "e");
        assert!(!sql.contains("UNION ALL"), "{sql}");
        assert_eq!(sql.matches("SELECT").count(), 1, "{sql}");
    }

    #[test]
    fn the_exists_check_matches_the_four_key_columns() {
        assert_eq!(
            generate_ledger_exists_sql("ds", "m", "{*}", "smelt.src", "2026-01-01"),
            "SELECT 1 FROM `ds._smelt_ledger` WHERE model_name = 'm' AND grp = '{*}' \
             AND input_name = 'smelt.src' AND delta_id = '2026-01-01' LIMIT 1"
        );
    }

    #[test]
    fn the_recompute_reset_deletes_every_intersecting_row_then_records_the_input_read() {
        let sqls = generate_ledger_recompute_reset_sqls(
            "ds",
            "m",
            "{*}",
            "2026-01-01",
            "2026-01-03",
            "smelt.src",
            "2026-01-01",
        );
        assert_eq!(sqls.len(), 2);
        assert_eq!(
            sqls[0],
            "DELETE FROM `ds._smelt_ledger` WHERE model_name = 'm' AND grp = '{*}' \
             AND region_start < '2026-01-03' AND region_end > '2026-01-01'"
        );
        assert_eq!(
            sqls[1],
            generate_ledger_insert_sql(
                "ds",
                "m",
                "{*}",
                "smelt.src",
                "2026-01-01",
                "2026-01-01",
                "2026-01-03"
            )
        );
    }

    /// GoogleSQL escaping is backslash-based, not DuckDB's quote doubling —
    /// `''` would terminate the literal here, and an unescaped backslash would
    /// silently change the recorded value.
    #[test]
    fn string_literals_use_googlesql_backslash_escaping() {
        let sql = generate_ledger_insert_sql("ds", "o'brien", "g", "a\\b", "d", "s", "e");
        assert!(sql.contains("'o\\'brien'"), "{sql}");
        assert!(sql.contains("'a\\\\b'"), "{sql}");
        assert!(!sql.contains("o''brien"), "{sql}");
    }

    /// A schema that already carries a project prefix stays one backticked
    /// path, matching `smelt_backend_bigquery::sql::qualified_name`.
    #[test]
    fn a_project_prefixed_schema_stays_one_backticked_path() {
        assert!(generate_ledger_table_ddl("my-proj.ds").contains("`my-proj.ds._smelt_ledger`"));
    }
}
