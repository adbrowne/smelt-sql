//! The succession grain's tombstone ledger table, GoogleSQL spelling.
//!
//! Split out of `ddl_bigquery/mod.rs` for the same reason the ledger and
//! observed-delta siblings were: one file per state structure, none of them a
//! token-cost hot spot.
//!
//! Bookkeeping DDL, in the same excluded class as the reconciliation ledger
//! and the observed-delta table (`CLAUDE.md` §"Maintenance-plan purity" —
//! "ledger DDL/DML in `smelt-state` excluded as bookkeeping"). The ledger's
//! *statements* are maintenance statements and are single-owned by
//! `smelt_logical::maintenance::emit::succession`; only the table's own
//! `CREATE`/`DROP` live here.
//!
//! Two GoogleSQL facts separate this from `ddl_duckdb`'s spelling:
//!
//! - **`PRIMARY KEY` must say `NOT ENFORCED`** — a bare `PRIMARY KEY` is a
//!   syntax error, and the declared key refuses nothing. The tombstone
//!   ledger never relies on it to refuse a duplicate: the idempotent
//!   tombstone insert is an anti-join (`NOT EXISTS`), authored in
//!   `smelt-logical`, which is blind to whether a key is enforced.
//! - **Column types are GoogleSQL's, not DuckDB's.** `DataType::to_sql()`
//!   renders `VARCHAR`/`INTEGER`, both `Type not found` in GoogleSQL, so
//!   every column routes through [`super::bigquery_type_sql`] — whose `Err`
//!   is propagated with the column named, never swallowed into a substitute
//!   type (`CLAUDE.md` §"Fail-loud discipline").

use smelt_types::DataType;

use super::bigquery_type_sql;

/// Backtick one already-joined `schema.table` path.
///
/// The caller holds the derived `<presented table>__tombstones` name as one
/// string (`smelt_logical::maintenance::emit::tombstone_table_name` owns the
/// suffix), so this is the one-argument sibling of `super::qualified` and
/// produces the same shape: the whole path backticked once, which keeps a
/// schema carrying a project prefix (`project.dataset`) working.
fn qualified_path(path: &str) -> String {
    format!("`{}`", path)
}

/// A tombstone-ledger column whose `DataType` has no GoogleSQL spelling.
///
/// Carries the column and the reason so the refusal names both, rather than
/// substituting a type the next write could not fill.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "tombstone ledger column '{column}' has no GoogleSQL type: {reason} \
     (docs/specs/state.md §\"Which dialects realise which structure\")"
)]
pub struct UnmappableTombstoneColumn {
    /// The offending column's own name.
    pub column: String,
    /// Why [`super::bigquery_type_sql`] refused the type.
    pub reason: String,
}

fn column_def(name: &str, ty: &DataType) -> Result<String, UnmappableTombstoneColumn> {
    let rendered = bigquery_type_sql(ty).map_err(|reason| UnmappableTombstoneColumn {
        column: name.to_string(),
        reason,
    })?;
    Ok(format!(
        "{} {} NOT NULL",
        super::quote_ident(name),
        rendered
    ))
}

/// DDL creating one model's tombstone ledger table, if it does not already
/// exist — the GoogleSQL sibling of
/// [`crate::ddl_duckdb::generate_tombstone_table_ddl`]. Same columns
/// (`key_cols ++ [clock_col]`, each `NOT NULL`), same key, same idempotence.
pub fn generate_tombstone_table_ddl(
    qualified_name: &str,
    key_cols: &[(String, DataType)],
    clock_col: &str,
    clock_type: &DataType,
) -> Result<String, UnmappableTombstoneColumn> {
    let mut col_defs: Vec<String> = key_cols
        .iter()
        .map(|(name, ty)| column_def(name, ty))
        .collect::<Result<Vec<_>, _>>()?;
    col_defs.push(column_def(clock_col, clock_type)?);
    let pk_cols = key_cols
        .iter()
        .map(|(name, _)| super::quote_ident(name))
        .chain(std::iter::once(super::quote_ident(clock_col)))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {} ({}, PRIMARY KEY ({pk_cols}) NOT ENFORCED)",
        qualified_path(qualified_name),
        col_defs.join(", ")
    ))
}

/// DDL dropping one model's tombstone ledger table — the GoogleSQL sibling
/// of [`crate::ddl_duckdb::generate_tombstone_table_drop_ddl`].
pub fn generate_tombstone_table_drop_ddl(qualified_name: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", qualified_path(qualified_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DataType {
        DataType::Timestamp {
            with_timezone: false,
        }
    }

    #[test]
    fn tombstone_ddl_is_googlesql_typed_backticked_and_not_enforced() {
        let sql = generate_tombstone_table_ddl(
            "smelt_dogfood.customer_history__tombstones",
            &[
                ("customer_id".to_string(), DataType::Integer),
                (
                    "region".to_string(),
                    DataType::Varchar {
                        max_length: Some(32),
                    },
                ),
            ],
            "changed_at",
            &ts(),
        )
        .expect("mappable types");
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS `smelt_dogfood.customer_history__tombstones` \
             (`customer_id` INT64 NOT NULL, `region` STRING NOT NULL, `changed_at` TIMESTAMP \
             NOT NULL, PRIMARY KEY (`customer_id`, `region`, `changed_at`) NOT ENFORCED)"
        );
        assert!(!sql.contains("VARCHAR"), "{sql}");
        assert!(!sql.contains('"'), "no double-quoted identifiers: {sql}");
    }

    #[test]
    fn an_unmappable_column_type_is_refused_by_name_never_substituted() {
        let err = generate_tombstone_table_ddl(
            "ds.t__tombstones",
            &[(
                "bad".to_string(),
                DataType::Map(Box::new(DataType::Text), Box::new(DataType::Text)),
            )],
            "changed_at",
            &ts(),
        )
        .expect_err("MAP has no GoogleSQL spelling");
        assert_eq!(err.column, "bad");
        assert!(err.to_string().contains("MAP"), "{err}");
    }

    #[test]
    fn drop_ddl_backticks_the_whole_path() {
        assert_eq!(
            generate_tombstone_table_drop_ddl("smelt_dogfood.customer_history__tombstones"),
            "DROP TABLE IF EXISTS `smelt_dogfood.customer_history__tombstones`"
        );
    }
}
