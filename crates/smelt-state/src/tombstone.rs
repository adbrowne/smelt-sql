//! The one dialect dispatch point for the succession grain's tombstone
//! ledger *table* (`docs/specs/incremental_shapes.md` §"The tombstone ledger
//! (hidden state)" — "Physical shape").
//!
//! Same shape, and the same reasons, as [`crate::ledger`] and
//! [`crate::observed_delta`]: every caller asks this module rather than
//! `ddl_duckdb::generate_tombstone_*` or `ddl_bigquery::generate_tombstone_*`
//! directly, so a dialect's spelling is chosen in exactly one place. The
//! `match` is exhaustive over [`SqlDialect`]: a new dialect is a compile
//! error here, never a silent default (`CLAUDE.md` §"Fail-loud discipline").
//!
//! **Only the table's own DDL lives here.** Every *statement* the tombstone
//! ledger participates in — the idempotent tombstone insert, the presented
//! `MERGE`, the rebuild's `DELETE`/`INSERT` pair, the clock-tie probe — is a
//! maintenance statement, single-owned by
//! `smelt_logical::maintenance::emit::succession` (`CLAUDE.md`
//! §"Maintenance-plan purity"; `docs/specs/incremental_models.md`
//! §"Statement emission (single owner)"). This module never authors one.
//!
//! **Spark is refused, not deferred** — same permanent absence the ledger
//! and observed-delta dispatches record, for the same reason: Delta has no
//! cross-table transaction, so the tombstone record and the presented write
//! cannot be made atomic.
//!
//! Realisability itself is **not** decided here —
//! `realisable_state_structures` in `smelt-logical` owns it, and a run-layer
//! caller consults that (via
//! `smelt_runtime::maintenance_driver::realises_tombstone_ledger`) before
//! building any statement. This module only answers "how is it spelled".

use smelt_dialect::SqlDialect;
use smelt_types::DataType;

use crate::{ddl_bigquery, ddl_duckdb};

/// Why a tombstone-ledger DDL could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TombstoneDdlError {
    /// A dialect with no tombstone-ledger spelling was asked for one.
    ///
    /// Reaching this means a caller skipped the availability check: the
    /// availability layer would have downgraded the cell to `DeleteInsert`
    /// and never got here.
    #[error(
        "no tombstone ledger DDL exists for dialect '{dialect}': the structure is not \
         realisable there, so the plan layer should have downgraded this cell before the run \
         reached a tombstone statement (docs/specs/state.md §\"Which dialects realise which \
         structure\")"
    )]
    UnsupportedDialect {
        /// The dialect's own name, as `SqlDialect::name` spells it.
        dialect: &'static str,
    },
    /// One of `key_cols ++ [clock_col]` has no spelling in the target
    /// dialect's type system. Named, never substituted.
    #[error("{0}")]
    UnmappableColumn(#[from] ddl_bigquery::UnmappableTombstoneColumn),
}

/// Idempotent DDL creating one model's tombstone ledger table if it does not
/// already exist.
///
/// `qualified_name` is the derived `<presented table>__tombstones` name;
/// `smelt_logical::maintenance::emit::tombstone_table_name` is the single
/// owner of that suffix, so it arrives as a parameter rather than being
/// re-derived below `smelt-logical` in the crate layering.
pub fn tombstone_table_ddl(
    dialect: SqlDialect,
    qualified_name: &str,
    key_cols: &[(String, DataType)],
    clock_col: &str,
    clock_type: &DataType,
) -> Result<String, TombstoneDdlError> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_tombstone_table_ddl(
            qualified_name,
            key_cols,
            clock_col,
            clock_type,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_tombstone_table_ddl(
            qualified_name,
            key_cols,
            clock_col,
            clock_type,
        )?),
        SqlDialect::SparkSQL => Err(TombstoneDdlError::UnsupportedDialect {
            dialect: dialect.name(),
        }),
    }
}

/// DDL dropping one model's tombstone ledger table — the same lifecycle
/// event as the presented table itself ("created with it, dropped with it").
pub fn tombstone_table_drop_ddl(
    dialect: SqlDialect,
    qualified_name: &str,
) -> Result<String, TombstoneDdlError> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_tombstone_table_drop_ddl(
            qualified_name,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_tombstone_table_drop_ddl(
            qualified_name,
        )),
        SqlDialect::SparkSQL => Err(TombstoneDdlError::UnsupportedDialect {
            dialect: dialect.name(),
        }),
    }
}
