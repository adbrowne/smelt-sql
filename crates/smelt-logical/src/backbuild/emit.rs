//! Pure statement emitters for backbuild's B1 (self-derivable column add)
//! and B2 (rename) techniques — the only statement-authoring surface for
//! those scripts (statement single-ownership,
//! `docs/specs/architecture.md` §"Constraints & Invariants" item 12).
//! Callers (`classify.rs`, the conformance harness) only ever execute the
//! strings these functions return; nothing outside this module composes
//! backbuild DDL/DML text.
//!
//! DuckDB-dialect, test-grade, per research
//! `docs/research/20260802-backbuild-synthesis.md` §3 ("DDL strings emitted
//! here are test-grade DuckDB dialect").

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
