//! Pure statement emitters for backbuild — the sole statement-authoring
//! surface for every technique's script (`FullRefresh`, B1–B7, D1–D2,
//! E1–E4, F1; statement single-ownership,
//! `docs/specs/architecture.md` §"Constraints & Invariants" item 12).
//! Callers (`classify.rs`, the conformance harness) only ever execute the
//! strings these functions return; nothing outside this module composes
//! backbuild DDL/DML text. Deliberately open-ended rather than an
//! enumerated per-technique list here — that list has gone stale before
//! (final-review batched finding: this doc comment once claimed only "B1
//! and B2" while the module had long since grown E-class/F1/B3/B4
//! emitters too).
//!
//! DuckDB-dialect, test-grade, per research
//! `docs/research/20260802-backbuild-synthesis.md` §3 ("DDL strings emitted
//! here are test-grade DuckDB dialect").

/// `CREATE OR REPLACE TABLE t AS <after>;` — the always-present model-level
/// `FullRefresh` baseline (research §2). Moved here from `classify.rs`'s
/// `full_refresh_option` (final-review batched finding) so every backbuild
/// statement, including the baseline every targeted script is compared
/// against, has exactly one authoring site.
pub fn emit_full_refresh(table: &str, after_sql: &str) -> String {
    format!("CREATE OR REPLACE TABLE {table} AS {after_sql}")
}

/// `ALTER TABLE t ADD COLUMN c <ty>;` — research §4 B1/B2/B3's first
/// H-slot step for every newly stored column.
pub fn emit_alter_add_column(table: &str, column: &str, sql_type: &str) -> String {
    format!("ALTER TABLE {table} ADD COLUMN {column} {sql_type}")
}

/// `ALTER TABLE t RENAME COLUMN d TO a;` — research §4 B2. Zero rows
/// touched.
pub fn emit_alter_rename_column(table: &str, from: &str, to: &str) -> String {
    format!("ALTER TABLE {table} RENAME COLUMN {from} TO {to}")
}

/// `ALTER TABLE t DROP COLUMN d;` — research §4 C1's sole statement, always
/// composed into the H "ALTER DROPs" slot (`HSlot::Drop`), last, after
/// every statement that might still read the column
/// (`docs/research/20260802-backbuild-synthesis.md` §4 "H. Composites").
pub fn emit_alter_drop_column(table: &str, column: &str) -> String {
    format!("ALTER TABLE {table} DROP COLUMN {column}")
}

/// `UPDATE t SET c1 = e1, c2 = e2, ...;` — the unregioned sibling of
/// `maintenance::emit::emit_in_place_update` (which requires a maintenance
/// `Region` bound for a partition-scoped backfill). Backbuild's B1/D1
/// self-read backfill touches every row unconditionally — there is no
/// region predicate to render — so this is its own, simpler emitter rather
/// than a fork of the maintenance one.
pub fn emit_in_place_update(table: &str, assignments: &[(String, String)]) -> String {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("UPDATE {table} SET {sets}")
}

/// `UPDATE t SET c1 = e1, ... FROM <upstream> u WHERE t.k1 = u.k1 AND
/// t.k2 = u.k2 ...` — research §4 B3/D2's column-scoped upstream backfill,
/// the "one admission path, two triggers" shape shared by an added column
/// pulling through an upstream (B3) and a changed column whose new
/// expression reads one (D2). `key_pairs` is `(target column name, upstream
/// column name)` per grain-link key component, ANDed for a composite key.
///
/// This is deliberately its own, simpler emitter rather than the
/// maintenance `emit_column_scoped_merge` shape: that emitter's `SET *`
/// source contract expects a full-row projection from the maintenance
/// region machinery, not backbuild's single/few-column, unregioned,
/// unledgered assignment list (research §4 B3).
pub fn emit_column_backfill_update_from(
    table: &str,
    assignments: &[(String, String)],
    upstream_physical: &str,
    upstream_alias: &str,
    key_pairs: &[(String, String)],
) -> String {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = key_pairs
        .iter()
        .map(|(t_col, u_col)| format!("{table}.{t_col} = {upstream_alias}.{u_col}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!("UPDATE {table} SET {sets} FROM {upstream_physical} {upstream_alias} WHERE {predicate}")
}

/// `UPDATE t SET c1 = s.c1, ... FROM (<subquery_sql>) s WHERE t.k1 = s.k1
/// AND ...` — research §4 B5's matched-only column backfill from a derived
/// subquery, rather than a bare upstream table
/// ([`emit_column_backfill_update_from`]'s `upstream_physical` is a plain
/// table reference; B5's source is instead the after-definition's own
/// re-aggregation — full `FROM`/`WHERE` tree, `GROUP BY` keys — so the
/// subquery text itself is wrapped in parens here, mirroring
/// [`emit_difference_insert`]'s `after_sql` treatment). **No insert arm**:
/// a group whose key does not already exist in `t` is a group the stored
/// row set proves cannot exist (the model's row-set-unchanged proof), so
/// only matched keys are ever updated (research §4 B5).
pub fn emit_column_backfill_update_from_subquery(
    table: &str,
    assignments: &[(String, String)],
    subquery_sql: &str,
    subquery_alias: &str,
    key_pairs: &[(String, String)],
) -> String {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = key_pairs
        .iter()
        .map(|(t_col, s_col)| format!("{table}.{t_col} = {subquery_alias}.{s_col}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!("UPDATE {table} SET {sets} FROM ({subquery_sql}) {subquery_alias} WHERE {predicate}")
}

/// `(SELECT <alias>.<dim_col> FROM <physical> <alias> WHERE <table>.<k1> =
/// <alias>.<k1> AND ...)` — research §4 B4's per-reference substituted
/// scalar-subquery shape: called once per dimension-column reference inside
/// an added expression (the caller, `classify.rs`, splices each returned
/// fragment into the expression via
/// `requalify::requalify_scalar_subquery`). A subquery that matches zero
/// dimension rows yields NULL for exactly this one reference — LEFT-JOIN
/// NULL-extension preserved at the leaf, while the rest of the surrounding
/// expression (e.g. a `COALESCE`) still runs; the naive whole-expression
/// form (wrapping the entire added expression in one subquery) is wrong for
/// exactly this reason (research §4 B4).
pub fn emit_scalar_subquery_fragment(
    table: &str,
    dim_col: &str,
    upstream_physical: &str,
    upstream_alias: &str,
    key_pairs: &[(String, String)],
) -> String {
    let predicate = key_pairs
        .iter()
        .map(|(t_col, u_col)| format!("{table}.{t_col} = {upstream_alias}.{u_col}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "(SELECT {upstream_alias}.{dim_col} FROM {upstream_physical} {upstream_alias} WHERE \
         {predicate})"
    )
}

/// `DELETE FROM t WHERE (<predicate>) IS NOT TRUE;` — research §4 E1's
/// filter-tightened `DELETE`. Three-valued logic is pinned here, at the
/// single authoring site: `IS NOT TRUE` deletes a row whose `predicate`
/// evaluates NULL, not just FALSE — a bare `NOT predicate` would wrongly
/// *keep* such a row (the regression trap research §4 E1 names explicitly).
pub fn emit_predicate_delete(table: &str, predicate: &str) -> String {
    format!("DELETE FROM {table} WHERE ({predicate}) IS NOT TRUE")
}

/// `INSERT INTO t (<cols>) SELECT <cols> FROM (<after_sql>) AS
/// __backbuild_diff WHERE (<difference_predicate>) IS NOT TRUE [AND NOT
/// EXISTS (<identity anti-join>)]` — research §4 E2/E4's region-scoped
/// difference `INSERT`.
///
/// `after_sql` is wrapped as a derived table rather than string-spliced into
/// (research §3 "Alias requalification is a CST rewrite, not string
/// surgery" — the same posture extended to whole-statement composition):
/// its own output columns are already named exactly like `table`'s stored
/// columns, so both `difference_predicate` and the identity anti-join guard
/// can address them by name without knowing anything about `after_sql`'s
/// own `FROM`/`JOIN` structure. `columns` is an explicit column list on
/// *both* the `INSERT` target and the `SELECT` — the research §4 E4
/// requirement — so column order only needs to agree between the two lists
/// here, never with `table`'s physical column order.
///
/// `difference_predicate` is E4's always-complement-form predicate (research
/// §4 E4: "the emitted predicate is always the complement form") — the
/// caller passes the bare removed/old conjunct text (already requalified to
/// stored-column names); this emitter applies the `IS NOT TRUE` wrapping,
/// the single authoring site for the three-valued-logic form, mirroring
/// [`emit_predicate_delete`].
///
/// `identity_columns` is `None` when `BackbuildInputs::row_identity` is
/// undeclared, or declared but not every identity column is provably NOT
/// NULL (`BackbuildInputs::not_null_columns` — an equality anti-join never
/// matches a NULL-keyed row, so a nullable identity column would let a
/// rerun silently re-insert; research §4 intro "Key addressability") — the
/// caller then records `rerun_safe: false` on the option this backs
/// (research §2 "Idempotence": "Plain INSERT-family steps … are not
/// [idempotent]; each carries an anti-join guard on row identity where
/// identity is available … and is otherwise documented one-shot").
pub fn emit_difference_insert(
    table: &str,
    columns: &[String],
    after_sql: &str,
    difference_predicate: &str,
    identity_columns: Option<&[String]>,
) -> String {
    let col_list = columns.join(", ");
    let mut sql = format!(
        "INSERT INTO {table} ({col_list}) SELECT {col_list} FROM ({after_sql}) AS \
         __backbuild_diff WHERE ({difference_predicate}) IS NOT TRUE"
    );
    if let Some(identity) = identity_columns {
        let guard = identity
            .iter()
            .map(|c| format!("{table}.{c} = __backbuild_diff.{c}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        sql.push_str(&format!(
            " AND NOT EXISTS (SELECT 1 FROM {table} WHERE {guard})"
        ));
    }
    sql
}

/// `INSERT INTO t (<cols>) SELECT <cols> FROM (<branch_sql>) AS
/// __backbuild_branch [WHERE NOT EXISTS (<identity anti-join>)]` — research
/// §4 F1's added-`UNION ALL`-branch `INSERT`. Unlike
/// [`emit_difference_insert`] (E2/E4), there is no difference predicate to
/// apply: `UNION ALL` is additive, so `branch_sql` — the added branch's own
/// text, verbatim — already *is* exactly the delta; the only optional guard
/// is the identity anti-join (same NOT-NULL-proven-identity conditionality
/// as E2/E4, research §4 intro "Key addressability"). `columns` is an
/// explicit column list on both the `INSERT` target and the `SELECT`
/// (research §4 "H. Composites": after an `ALTER ADD`, the table's physical
/// column order differs from the after-definition's declared order, so a
/// positional `INSERT` would silently misassign same-typed columns).
pub fn emit_branch_insert(
    table: &str,
    columns: &[String],
    branch_sql: &str,
    identity_columns: Option<&[String]>,
) -> String {
    let col_list = columns.join(", ");
    let mut sql = format!(
        "INSERT INTO {table} ({col_list}) SELECT {col_list} FROM ({branch_sql}) AS \
         __backbuild_branch"
    );
    if let Some(identity) = identity_columns {
        let guard = identity
            .iter()
            .map(|c| format!("{table}.{c} = __backbuild_branch.{c}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        sql.push_str(&format!(
            " WHERE NOT EXISTS (SELECT 1 FROM {table} WHERE {guard})"
        ));
    }
    sql
}

/// `DELETE FROM t WHERE <column> = <literal>;` — research §4 F2's
/// discriminated-branch-removal `DELETE`. An **equality** predicate, unlike
/// [`emit_predicate_delete`]'s `IS NOT TRUE` three-valued-logic form:
/// `classify.rs`'s `find_branch_discriminator` only ever supplies a
/// `column`/`literal` pair it has already proven is (a) a bare, non-NULL
/// literal — so there is no NULL-evaluation case to guard against — and (b)
/// distinct from every surviving branch's own constant for that column, so
/// the predicate provably matches only the removed branch's rows. `literal`
/// is caller-rendered SQL text (already quoted for a text literal, bare for
/// a numeric one) — this emitter only splices it in, the single authoring
/// site for the statement shape.
pub fn emit_discriminated_branch_delete(table: &str, column: &str, literal: &str) -> String {
    format!("DELETE FROM {table} WHERE {column} = {literal}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_refresh_shape() {
        assert_eq!(
            emit_full_refresh("t", "SELECT id FROM orders"),
            "CREATE OR REPLACE TABLE t AS SELECT id FROM orders"
        );
    }

    #[test]
    fn alter_add_column_shape() {
        assert_eq!(
            emit_alter_add_column("t", "status", "TEXT"),
            "ALTER TABLE t ADD COLUMN status TEXT"
        );
    }

    #[test]
    fn alter_rename_column_shape() {
        assert_eq!(
            emit_alter_rename_column("t", "old_name", "new_name"),
            "ALTER TABLE t RENAME COLUMN old_name TO new_name"
        );
    }

    #[test]
    fn alter_drop_column_shape() {
        assert_eq!(
            emit_alter_drop_column("t", "extra"),
            "ALTER TABLE t DROP COLUMN extra"
        );
    }

    #[test]
    fn in_place_update_shape_single_assignment() {
        assert_eq!(
            emit_in_place_update("t", &[("total".to_string(), "price * qty".to_string())]),
            "UPDATE t SET total = price * qty"
        );
    }

    #[test]
    fn in_place_update_shape_multiple_assignments() {
        assert_eq!(
            emit_in_place_update(
                "t",
                &[
                    ("a".to_string(), "1".to_string()),
                    ("b".to_string(), "2".to_string()),
                ]
            ),
            "UPDATE t SET a = 1, b = 2"
        );
    }

    #[test]
    fn column_backfill_update_from_shape_single_key() {
        assert_eq!(
            emit_column_backfill_update_from(
                "t",
                &[("discount".to_string(), "u.discount".to_string())],
                "orders",
                "u",
                &[("order_id".to_string(), "order_id".to_string())],
            ),
            "UPDATE t SET discount = u.discount FROM orders u WHERE t.order_id = u.order_id"
        );
    }

    #[test]
    fn column_backfill_update_from_shape_composite_key() {
        assert_eq!(
            emit_column_backfill_update_from(
                "t",
                &[("total".to_string(), "u.amount".to_string())],
                "orders",
                "u",
                &[
                    ("region".to_string(), "region".to_string()),
                    ("order_id".to_string(), "order_id".to_string()),
                ],
            ),
            "UPDATE t SET total = u.amount FROM orders u WHERE t.region = u.region AND \
             t.order_id = u.order_id"
        );
    }

    #[test]
    fn column_backfill_update_from_subquery_shape_single_key() {
        assert_eq!(
            emit_column_backfill_update_from_subquery(
                "t",
                &[("total_qty".to_string(), "s.total_qty".to_string())],
                "SELECT o.customer_id AS customer_id, SUM(o.qty) AS total_qty FROM orders o \
                 GROUP BY o.customer_id",
                "s",
                &[("customer_id".to_string(), "customer_id".to_string())],
            ),
            "UPDATE t SET total_qty = s.total_qty FROM (SELECT o.customer_id AS customer_id, \
             SUM(o.qty) AS total_qty FROM orders o GROUP BY o.customer_id) s WHERE \
             t.customer_id = s.customer_id"
        );
    }

    #[test]
    fn scalar_subquery_fragment_shape_single_key() {
        assert_eq!(
            emit_scalar_subquery_fragment(
                "t",
                "customer_name",
                "customers",
                "c",
                &[("customer_id".to_string(), "customer_id".to_string())],
            ),
            "(SELECT c.customer_name FROM customers c WHERE t.customer_id = c.customer_id)"
        );
    }

    #[test]
    fn scalar_subquery_fragment_shape_composite_key() {
        assert_eq!(
            emit_scalar_subquery_fragment(
                "t",
                "name",
                "dims",
                "d",
                &[
                    ("region".to_string(), "region".to_string()),
                    ("dim_id".to_string(), "dim_id".to_string()),
                ],
            ),
            "(SELECT d.name FROM dims d WHERE t.region = d.region AND t.dim_id = d.dim_id)"
        );
    }

    #[test]
    fn predicate_delete_shape() {
        assert_eq!(
            emit_predicate_delete("t", "status = 'active'"),
            "DELETE FROM t WHERE (status = 'active') IS NOT TRUE"
        );
    }

    #[test]
    fn difference_insert_shape_without_identity() {
        assert_eq!(
            emit_difference_insert(
                "t",
                &["ts".to_string(), "amount".to_string()],
                "SELECT ts, amount FROM events WHERE ts >= '2024-01-01'",
                "ts >= '2025-01-01'",
                None,
            ),
            "INSERT INTO t (ts, amount) SELECT ts, amount FROM (SELECT ts, amount FROM events \
             WHERE ts >= '2024-01-01') AS __backbuild_diff WHERE (ts >= '2025-01-01') IS NOT \
             TRUE"
        );
    }

    #[test]
    fn difference_insert_shape_with_identity_guard() {
        assert_eq!(
            emit_difference_insert(
                "t",
                &["id".to_string(), "ts".to_string()],
                "SELECT id, ts FROM events WHERE ts >= '2024-01-01'",
                "ts >= '2025-01-01'",
                Some(&["id".to_string()]),
            ),
            "INSERT INTO t (id, ts) SELECT id, ts FROM (SELECT id, ts FROM events WHERE ts >= \
             '2024-01-01') AS __backbuild_diff WHERE (ts >= '2025-01-01') IS NOT TRUE AND NOT \
             EXISTS (SELECT 1 FROM t WHERE t.id = __backbuild_diff.id)"
        );
    }

    #[test]
    fn difference_insert_shape_composite_identity_guard() {
        assert_eq!(
            emit_difference_insert(
                "t",
                &["region".to_string(), "ts".to_string()],
                "SELECT region, ts FROM events WHERE ts >= '2024-01-01'",
                "ts >= '2025-01-01'",
                Some(&["region".to_string(), "ts".to_string()]),
            ),
            "INSERT INTO t (region, ts) SELECT region, ts FROM (SELECT region, ts FROM events \
             WHERE ts >= '2024-01-01') AS __backbuild_diff WHERE (ts >= '2025-01-01') IS NOT \
             TRUE AND NOT EXISTS (SELECT 1 FROM t WHERE t.region = __backbuild_diff.region AND \
             t.ts = __backbuild_diff.ts)"
        );
    }

    #[test]
    fn branch_insert_shape_without_identity() {
        assert_eq!(
            emit_branch_insert(
                "t",
                &["id".to_string(), "kind".to_string()],
                "SELECT id, 'b' AS kind FROM events_b",
                None,
            ),
            "INSERT INTO t (id, kind) SELECT id, kind FROM (SELECT id, 'b' AS kind FROM \
             events_b) AS __backbuild_branch"
        );
    }

    #[test]
    fn branch_insert_shape_with_identity_guard() {
        assert_eq!(
            emit_branch_insert(
                "t",
                &["id".to_string(), "kind".to_string()],
                "SELECT id, 'b' AS kind FROM events_b",
                Some(&["id".to_string()]),
            ),
            "INSERT INTO t (id, kind) SELECT id, kind FROM (SELECT id, 'b' AS kind FROM \
             events_b) AS __backbuild_branch WHERE NOT EXISTS (SELECT 1 FROM t WHERE t.id = \
             __backbuild_branch.id)"
        );
    }

    #[test]
    fn discriminated_branch_delete_shape_text_literal() {
        assert_eq!(
            emit_discriminated_branch_delete("t", "src", "'b'"),
            "DELETE FROM t WHERE src = 'b'"
        );
    }

    #[test]
    fn discriminated_branch_delete_shape_number_literal() {
        assert_eq!(
            emit_discriminated_branch_delete("t", "src", "2"),
            "DELETE FROM t WHERE src = 2"
        );
    }
}
