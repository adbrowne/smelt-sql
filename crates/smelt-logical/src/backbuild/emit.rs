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
}
