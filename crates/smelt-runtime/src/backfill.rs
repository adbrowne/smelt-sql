//! Targeted column backfill — the mechanism behind the "Targeted column
//! backfill" transform.
//!
//! See `docs/specs/model_transforms.md` §Surface "Targeted column backfill"
//! and §Semantics "Targeted column backfill and dimension-driven horizon
//! MERGE". Backfill is licensed by an *additive-only model diff*
//! (`smelt_logical::analysis::model_diff::additive_only_diff`) — an edit
//! that only adds columns derivable from `{existing target} ∪ {monotone
//! dimension}` — and edits those columns **in place** via an `UPDATE ...
//! FROM (...) AS src` statement, never a full rebuild.
//!
//! Fail-closed (`model_transforms.md` §Constraints "Equivalence or
//! refusal"): the moment the diff is not additive-only, or there is no
//! `unique_key` to join existing rows against the recomputed source, this
//! refuses before emitting any SQL — a backfill without a key to match rows
//! by cannot be a *targeted* edit; it would have to touch every row,
//! indistinguishable from a rebuild.

use smelt_logical::analysis::model_diff::{ColumnDef, ModelDiff};

/// Build the `UPDATE ... FROM (...) AS src WHERE ...` statement that
/// back-fills only the columns added by an additive-only model diff, or
/// refuse with a reason when the edit is not licensed for a targeted
/// backfill.
///
/// `table` is the schema-qualified target table name. `recompute_select_sql`
/// is the model's full recomputed `SELECT` body — the source of truth for
/// the added columns' values; this function wraps it in a subquery that
/// projects only `unique_key` plus the added columns, since that is the
/// minimal recompute needed for the backfill.
///
/// Refuses (returns `Err`) when:
/// - `diff` is [`ModelDiff::NotAdditive`] — its reason is propagated.
/// - `unique_key` is empty — there is no key to target existing rows by.
pub fn targeted_column_backfill(
    diff: &ModelDiff,
    old_columns: &[ColumnDef],
    new_columns: &[ColumnDef],
    unique_key: &[String],
    table: &str,
    recompute_select_sql: &str,
) -> Result<String, String> {
    match diff {
        ModelDiff::NotAdditive { reason } => {
            return Err(format!(
                "targeted column backfill refused for table '{table}': model diff is not \
                 additive-only: {reason}"
            ));
        }
        ModelDiff::AdditiveOnly => {}
    }

    if unique_key.is_empty() {
        return Err(format!(
            "targeted column backfill refused for table '{table}': no unique_key declared — a \
             backfill without a key to match existing rows by cannot be targeted, it would have \
             to touch every row, indistinguishable from a rebuild"
        ));
    }

    let old_names: std::collections::HashSet<&str> =
        old_columns.iter().map(|c| c.name.as_str()).collect();
    let added_columns: Vec<&str> = new_columns
        .iter()
        .map(|c| c.name.as_str())
        .filter(|name| !old_names.contains(name))
        .collect();

    if added_columns.is_empty() {
        return Err(format!(
            "targeted column backfill refused for table '{table}': additive-only diff added no \
             new columns — nothing to backfill"
        ));
    }

    // Minimal recompute: unique_key columns plus the added columns only.
    let mut projected: Vec<&str> = unique_key.iter().map(|k| k.as_str()).collect();
    projected.extend(added_columns.iter().copied());
    let projection = projected.join(", ");

    let set_clause = added_columns
        .iter()
        .map(|col| format!("{col} = src.{col}"))
        .collect::<Vec<_>>()
        .join(", ");

    let join_clause = unique_key
        .iter()
        .map(|key| format!("{table}.{key} = src.{key}"))
        .collect::<Vec<_>>()
        .join(" AND ");

    Ok(format!(
        "UPDATE {table} SET {set_clause} FROM (SELECT {projection} FROM ({recompute_select_sql}) \
         AS recomputed) AS src WHERE {join_clause}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> ColumnDef {
        // The backfill function only needs `name`; build a trivial column
        // definition via the shared test-only `col` helper isn't exposed by
        // model_diff, so construct via a real parsed expression here too —
        // reuse the same "SELECT <expr> AS v FROM t" trick.
        let sql = format!("SELECT {name} AS v FROM t");
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
    fn additive_diff_with_unique_key_emits_targeted_update() {
        let old = vec![col("amount"), col("user_id")];
        let new = vec![col("amount"), col("user_id"), col("amount_usd")];
        let diff = ModelDiff::AdditiveOnly;

        let sql = targeted_column_backfill(
            &diff,
            &old,
            &new,
            &["user_id".to_string()],
            "main.orders",
            "SELECT amount, user_id, amount * fx_rate AS amount_usd FROM main.orders_source",
        )
        .expect("licensed backfill should succeed");

        assert!(sql.contains("UPDATE main.orders"));
        assert!(sql.contains("SET amount_usd = src.amount_usd"));
        assert!(sql.contains("FROM (SELECT user_id, amount_usd FROM ("));
        assert!(sql.contains("main.orders.user_id = src.user_id"));
        // Only the added column is touched, not the pre-existing ones.
        assert!(!sql.contains("SET amount = src.amount"));
        // Never a full rebuild.
        assert!(!sql.to_uppercase().contains("CREATE"));
    }

    #[test]
    fn not_additive_diff_is_refused() {
        let old = vec![col("amount")];
        let new = vec![col("amount")];
        let diff = ModelDiff::NotAdditive {
            reason: "existing column 'amount' changed its expression".to_string(),
        };

        let err = targeted_column_backfill(
            &diff,
            &old,
            &new,
            &["user_id".to_string()],
            "main.orders",
            "SELECT amount FROM main.orders_source",
        )
        .expect_err("non-additive diff must be refused");

        assert!(err.contains("existing column 'amount' changed its expression"));
        assert!(!err.to_uppercase().contains("UPDATE MAIN.ORDERS SET"));
    }

    #[test]
    fn additive_diff_without_unique_key_is_refused() {
        let old = vec![col("amount")];
        let new = vec![col("amount"), col("amount_usd")];
        let diff = ModelDiff::AdditiveOnly;

        let err = targeted_column_backfill(
            &diff,
            &old,
            &new,
            &[],
            "main.orders",
            "SELECT amount, amount * fx_rate AS amount_usd FROM main.orders_source",
        )
        .expect_err("empty unique_key must be refused");

        assert!(err.contains("no unique_key declared"));
        assert!(!err.to_uppercase().contains("UPDATE MAIN.ORDERS SET"));
    }
}
