//! F14 — targeted column backfill equivalence fixture.
//!
//! Proves the SQL emitted by `smelt_runtime::targeted_column_backfill`
//! satisfies the `maintenance_plan.md` §"The equivalence invariant" oracle:
//! running the emitted `UPDATE ... FROM (...) AS src` against a table with
//! pre-existing rows produces exactly the same result — row for row, on the
//! added column's values — as a full rebuild (`CREATE OR REPLACE TABLE ...
//! AS SELECT ...` including the new column) over the same source data.
//!
//! Requires `DUCKDB_LIB_DIR`/system DuckDB (`duckdb` feature, the default);
//! guarded the same way `incremental_parity.rs` guards its DuckDB path.

#![cfg(feature = "duckdb")]

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::model_diff::{additive_only_diff, ColumnDef, ModelDiff};
use smelt_runtime::targeted_column_backfill;
use tempfile::TempDir;

/// Build a `ColumnDef` by parsing a trivial `SELECT <expr> AS v FROM t` and
/// pulling the single select-item expression back out — mirrors the helper
/// `model_diff.rs`'s own tests use.
fn col(name: &str, sql_expr: &str) -> ColumnDef {
    let sql = format!("SELECT {sql_expr} AS v FROM t");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select");
    let item = select
        .select_list()
        .expect("select list")
        .items()
        .next()
        .expect("item");
    ColumnDef {
        name: name.to_string(),
        expr: item.expression().expect("expression"),
    }
}

#[test]
fn backfill_update_matches_full_rebuild() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let schema = "main";
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        let backend = DuckDbBackend::new(&db_path, schema)
            .await
            .unwrap_or_else(|e| panic!("DuckDB open failed: {e}"));

        // Source table the model reads from: order_id, user_id, amount, fx_rate.
        backend
            .execute_sql(
                "CREATE TABLE orders_source AS \
                 SELECT 1 AS order_id, 'A' AS user_id, 100 AS amount, 1.1 AS fx_rate \
                 UNION ALL SELECT 2, 'B', 200, 1.2 \
                 UNION ALL SELECT 3, 'A', 300, 1.3",
            )
            .await
            .unwrap_or_else(|e| panic!("create orders_source failed: {e}"));

        // Existing target table (as if from a prior run, before amount_usd existed).
        backend
            .execute_sql(
                "CREATE TABLE orders AS \
                 SELECT order_id, user_id, amount FROM orders_source",
            )
            .await
            .unwrap_or_else(|e| panic!("create orders (prior run) failed: {e}"));

        // Model diff: old model selected order_id/user_id/amount; new model adds
        // amount_usd, derivable from the existing `amount` column plus the
        // monotone dimension `fx_rate`.
        let old_columns = vec![
            col("order_id", "order_id"),
            col("user_id", "user_id"),
            col("amount", "amount"),
        ];
        let new_columns = vec![
            col("order_id", "order_id"),
            col("user_id", "user_id"),
            col("amount", "amount"),
            col("amount_usd", "amount * fx_rate"),
        ];
        let diff = additive_only_diff(&old_columns, &new_columns, &["fx_rate".to_string()]);
        assert_eq!(diff, ModelDiff::AdditiveOnly);

        let recompute_select_sql =
            "SELECT order_id, user_id, amount, amount * fx_rate AS amount_usd FROM orders_source";

        let update_sql = targeted_column_backfill(
            &diff,
            &old_columns,
            &new_columns,
            &["order_id".to_string()],
            "orders",
            recompute_select_sql,
        )
        .expect("licensed backfill should succeed");

        assert!(update_sql.contains("UPDATE orders"));
        assert!(!update_sql.to_uppercase().contains("CREATE"));

        // First: add the new column with NULLs so the UPDATE has somewhere to
        // write (this UPDATE only sets existing columns' values, it does not
        // ALTER the schema — that's a separate, out-of-scope concern for this
        // phase).
        backend
            .execute_sql("ALTER TABLE orders ADD COLUMN amount_usd DOUBLE")
            .await
            .unwrap_or_else(|e| panic!("ALTER TABLE failed: {e}"));

        backend
            .execute_sql(&update_sql)
            .await
            .unwrap_or_else(|e| panic!("backfill UPDATE failed: {e}\nSQL: {update_sql}"));

        // Oracle: full rebuild over the same source data, including the new column.
        backend
            .execute_sql(
                "CREATE OR REPLACE TABLE orders_rebuilt AS \
                 SELECT order_id, user_id, amount, amount * fx_rate AS amount_usd \
                 FROM orders_source",
            )
            .await
            .unwrap_or_else(|e| panic!("rebuild failed: {e}"));

        let backfilled = backend
            .execute_sql("SELECT order_id, amount_usd FROM orders ORDER BY order_id")
            .await
            .unwrap_or_else(|e| panic!("select backfilled failed: {e}"));
        let rebuilt = backend
            .execute_sql("SELECT order_id, amount_usd FROM orders_rebuilt ORDER BY order_id")
            .await
            .unwrap_or_else(|e| panic!("select rebuilt failed: {e}"));

        let backfilled_rows = batches_to_strings(&backfilled);
        let rebuilt_rows = batches_to_strings(&rebuilt);

        assert_eq!(
            backfilled_rows, rebuilt_rows,
            "targeted backfill result must equal full rebuild result"
        );
    });
}

#[test]
fn not_additive_diff_never_touches_the_table() {
    let old_columns = vec![col("amount", "amount")];
    let new_columns = vec![col("amount", "amount * 2")];
    let diff = additive_only_diff(&old_columns, &new_columns, &[]);
    assert!(!diff.is_additive_only());

    let err = targeted_column_backfill(
        &diff,
        &old_columns,
        &new_columns,
        &["order_id".to_string()],
        "orders",
        "SELECT amount * 2 AS amount FROM orders_source",
    )
    .expect_err("non-additive diff must be refused, never emit an UPDATE");
    assert!(!err.to_uppercase().contains("UPDATE ORDERS SET"));
}

/// Render every batch's rows as comparable strings (column-major arrow
/// pretty-print would work too, but a plain debug string is enough for a
/// row-for-row equality oracle here).
fn batches_to_strings(batches: &[arrow::array::RecordBatch]) -> String {
    use arrow::util::pretty::pretty_format_batches;
    pretty_format_batches(batches)
        .map(|d| d.to_string())
        .unwrap_or_default()
}
