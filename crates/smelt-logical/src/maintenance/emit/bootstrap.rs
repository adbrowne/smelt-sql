//! First-run table bootstrap statements: `CREATE TABLE … AS SELECT` and
//! the empty-table create, plus the dialect-keyed column type rendering
//! they share.

use super::types::*;

/// First-run `CREATE TABLE … AS` for a windowed-keyed-maintenance cell
/// (`maintenance_driver::run_windowed_keyed_maintenance`'s create arm): the
/// target table does not exist yet, so the first step's delta becomes the
/// table wholesale — no read of prior state, no `MERGE`.
///
/// `table` is already fully qualified (`schema.table`); `select_sql` is the
/// caller's already-compiled delta `SELECT` for that step, unmodified.
///
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; `CREATE TABLE … AS SELECT …` is dialect-invariant across
/// DuckDB and Spark for this family.
pub fn emit_create_table_as(
    table: &str,
    select_sql: &str,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    // Every step after this bootstrap CREATE for a merge-based cell (keyed
    // fold, column-scoped merge) is a `MERGE INTO`, which Spark refuses
    // against a plain (default-format, non-Delta) managed table — so the
    // bootstrap itself must specify `USING DELTA` on Spark. DuckDB has no
    // such format clause. `MaintenanceDialect::Spark` covers only the
    // merge-capable Spark family (see `smelt_backend::maintenance_dialect`),
    // so this is never reached for the cross-engine Parquet read path
    // (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 4).
    let using_clause = match dialect {
        MaintenanceDialect::DuckDb => "",
        MaintenanceDialect::Spark => " USING DELTA",
        // BigQuery has one table format and no format clause; MERGE against a
        // plainly-created table is accepted, so the bootstrap needs nothing
        // extra. Confirmed live (scripts/bigquery-probe3.sh).
        MaintenanceDialect::BigQuery => "",
    };
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "CREATE TABLE {table}{using_clause} AS {select_sql}"
        ))],
        transactional: false,
    }
}

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_shapes.md` §"First-run and backfill" —
/// "First-run bootstrap for a self-referential model"): the target does not
/// exist yet, and the model's own first-batch SELECT reads that same target
/// via `smelt.<self>`, so `CREATE TABLE … AS SELECT …` cannot resolve it —
/// no engine can create a table and read it in the same statement. Instead
/// this emitter authors a plain, empty `CREATE TABLE` from the caller's own
/// inferred output schema (column name, SQL type) — no `SELECT`, no read of
/// any table. The subsequent batch loop then executes the model's first
/// partition (and every later one) as the ordinary region `DELETE`+`INSERT`
/// (`emit_delete_insert`); the self-read over this empty table correctly
/// sees no prior state.
///
/// `columns` is plain data the caller already resolved — a self-
/// referential model's own output-schema fixpoint
/// (`smelt-runtime`'s `UpstreamSchemas::from_database`, which refines what
/// `resolved_model_schema` alone cannot: that Salsa query's `cycle_initial`
/// BREAKS a genuine self-referential cycle with an empty schema rather than
/// iterating it to a fixpoint). This function does no inference of its own,
/// only string assembly, preserving the maintenance-plan purity invariant
/// (`docs/specs/architecture.md` §"Constraints & Invariants" item 12).
///
/// `table` is already fully qualified (`schema.table`). `dialect` selects
/// the SQL type spelling for `Text`/`Varchar` columns: Spark 4+ rejects a
/// bare `VARCHAR` (no length), so those columns render as `STRING`; DuckDB
/// takes bare `VARCHAR`. Every other `DataType` renders via
/// `DataType::to_backend_sql()`, the same mapping `wrap_with_type_casts`
/// (`smelt-dialect`) uses for its CAST wrapper.
///
/// # Panics
/// Panics if `columns` is empty — a model with no resolvable output columns
/// cannot be bootstrapped into a valid `CREATE TABLE`; the caller must not
/// reach this emitter without a non-empty resolved schema.
pub fn emit_create_empty_table(
    table: &str,
    columns: &[(String, smelt_types::DataType)],
    dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !columns.is_empty(),
        "emit_create_empty_table requires a non-empty resolved output schema for {table}"
    );
    let col_defs = columns
        .iter()
        .map(|(name, dt)| format!("{name} {}", bootstrap_column_sql_type(dt, dialect)))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "CREATE TABLE {table} ({col_defs})"
        ))],
        transactional: false,
    }
}

/// The SQL type spelling `emit_create_empty_table` uses for one column.
/// Mirrors `smelt_dialect::type_conformance::type_cast_sql`'s Spark-STRING
/// carve-out (that function is not reused directly: `smelt-logical` and
/// `smelt-dialect` are sibling crates over `smelt-types`/`smelt-parser`, and
/// this module's dependency footprint is deliberately kept to `smelt-types`
/// alone, matching every other emitter in this file).
fn bootstrap_column_sql_type(dt: &smelt_types::DataType, dialect: MaintenanceDialect) -> String {
    match (dt, dialect) {
        (smelt_types::DataType::Text, MaintenanceDialect::Spark)
        | (smelt_types::DataType::Varchar { max_length: None }, MaintenanceDialect::Spark) => {
            "STRING".to_string()
        }
        // GoogleSQL rejects the string and floating-point names `to_backend_sql`
        // emits — VARCHAR, TEXT, DOUBLE, REAL and FLOAT are all `Type not found`
        // (verified by scripts/bigquery-probe4.sh, which also confirms the integer
        // aliases, DECIMAL, TIMESTAMP and DATE are accepted verbatim). Only the
        // rejected families are rewritten here; the rest pass through unchanged.
        (dt, MaintenanceDialect::BigQuery) => match dt {
            smelt_types::DataType::Text
            | smelt_types::DataType::Varchar { .. }
            | smelt_types::DataType::Char { .. } => "STRING".to_string(),
            smelt_types::DataType::Float | smelt_types::DataType::Double => "FLOAT64".to_string(),
            smelt_types::DataType::Blob => "BYTES".to_string(),
            other => other.to_backend_sql(),
        },
        _ => dt.to_backend_sql(),
    }
}
