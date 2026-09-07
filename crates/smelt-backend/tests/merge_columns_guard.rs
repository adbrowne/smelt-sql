//! `require_merge_columns` — the guard standing between a model with an
//! unresolvable projection and a BigQuery `MERGE` that silently stops
//! updating rows.
//!
//! The failure it prevents is not a crash. GoogleSQL has no `UPDATE SET *`,
//! so an empty column list renders a *syntactically valid* `MERGE` whose
//! matched arm assigns nothing: the statement succeeds, the job reports no
//! error, and matched rows quietly keep their old values. That is why the
//! guard exists rather than letting the warehouse reject the SQL, and why it
//! is worth pinning — a refactor that drops it would surface as wrong data on
//! one backend, not as a failing build.
//!
//! Reachability is real, not theoretical: a wildcard select is not
//! enumerable, so `CompiledModel::output_columns` is empty for a
//! `SELECT *` model (`smelt-runtime`'s
//! `projection_source_derived::bare_wildcard_projection_still_yields_empty_output_columns`),
//! and nothing between the projection and the write reads that field — so
//! such a model carrying a `unique_key` resolves to
//! `Technique::ColumnScopedMerge` and arrives here. See
//! `docs/specs/multi_backend.md` §Known Divergences.

use smelt_backend::require_merge_columns;
use smelt_dialect::SqlDialect;

const SCHEMA: &str = "analytics";
const TABLE: &str = "dim_customer";

#[test]
fn bigquery_refuses_a_whole_row_merge_with_no_column_list() {
    let err = require_merge_columns(SqlDialect::BigQuery, SCHEMA, TABLE, &[])
        .expect_err("BigQuery has no `UPDATE SET *`, so an empty column list must be refused");

    // The message must name the model — an operator seeing this needs to know
    // *which* model to add a projection to, and fail-loud discipline
    // (`architecture.md` §"Fail-loud discipline") makes naming it the point.
    let msg = err.to_string();
    assert!(
        msg.contains("analytics.dim_customer"),
        "the refusal must name the model it refused; got: {msg}"
    );
}

#[test]
fn bigquery_admits_a_whole_row_merge_that_has_a_column_list() {
    let columns = vec!["id".to_string(), "name".to_string()];
    assert!(
        require_merge_columns(SqlDialect::BigQuery, SCHEMA, TABLE, &columns).is_ok(),
        "a resolved column list is exactly what BigQuery needs — it must pass"
    );
}

/// The star dialects never read `columns`, so an empty list is not a defect
/// there. Asserting this is what keeps the guard *narrow*: widening it to
/// every dialect would refuse `SELECT *` models that DuckDB and Spark handle
/// correctly today, turning a BigQuery-only limitation into a global one.
#[test]
fn star_dialects_accept_an_empty_column_list() {
    for dialect in [SqlDialect::DuckDB, SqlDialect::SparkSQL] {
        assert!(
            require_merge_columns(dialect, SCHEMA, TABLE, &[]).is_ok(),
            "{dialect:?} spells the matched arm `UPDATE SET *` and never reads the column \
             list, so an empty list must not be refused"
        );
    }
}
