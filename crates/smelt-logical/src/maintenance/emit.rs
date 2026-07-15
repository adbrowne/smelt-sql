//! Physical maintenance SQL emission — the single author of every
//! maintenance statement a run executes
//! (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
//!
//! One emitter per [`Technique`](super::Technique), following the
//! physical-maintenance notation of
//! `docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`:
//! the partition predicate is carried on **both** the scan and the write
//! target wherever the op is region-scoped — a predicate stated only on one
//! side is a logical bound the storage layer cannot use
//! (`01-framework.md` §5).
//!
//! Emission is pure string construction over a caller-supplied SELECT body
//! (the model SQL with source refs resolved to physical table names); clamp
//! *injection into* the body is the runtime transformer's job
//! (`smelt-runtime/src/transformer.rs`) and is deliberately not duplicated
//! here — an emitter never adds a predicate the caller did not already fold
//! into the body it hands in, so the emitted text is exactly what a backend
//! executes, byte for byte.
//!
//! Backends *execute* the [`StatementGroup`]s these functions return; they
//! never author maintenance-statement text of their own
//! (`docs/specs/architecture.md` §"Constraints & Invariants" item 12).

use super::ScanClamp;

/// One SQL statement a maintenance run executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceStatement {
    pub sql: String,
}

impl MaintenanceStatement {
    fn new(sql: String) -> Self {
        Self { sql }
    }
}

/// An ordered group of [`MaintenanceStatement`]s produced by one emitter
/// call, plus whether they must run inside a single backend transaction. A
/// paired region `DELETE`+`INSERT` is transactional: a failed `INSERT` must
/// roll back its `DELETE` (`docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementGroup {
    pub statements: Vec<MaintenanceStatement>,
    pub transactional: bool,
}

/// The backend SQL dialect a [`StatementGroup`] is rendered for. Dialect
/// differences (e.g. a `MERGE … UPDATE SET *` requiring a full-row source
/// projection versus an explicit column-list `SET`) live in the emitters as
/// dialect-keyed variants, not in backend string construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceDialect {
    DuckDb,
    Spark,
}

/// A half-open region `[start, end)` on the output partition column; values
/// are SQL literals (already quoted where needed).
#[derive(Debug, Clone)]
pub struct Region {
    pub start: String,
    pub end: String,
}

/// The widened scan predicate a derived [`ScanClamp`] implies for
/// maintaining output region `[start, end)`: the source's partition column
/// over `[start − before, end + after)`. This is the *derived* number turned
/// into SQL — the caller injects it into the read (the body's source scan),
/// so a wrongly-derived window fails the equivalence oracle rather than
/// silently over- or under-reading.
pub fn widened_scan_predicate(clamp: &ScanClamp, region: &Region) -> String {
    let lower = if clamp.before.0 == 0 {
        region.start.clone()
    } else {
        format!("{} - INTERVAL '{} seconds'", region.start, clamp.before.0)
    };
    let upper = if clamp.after.0 == 0 {
        region.end.clone()
    } else {
        format!("{} + INTERVAL '{} seconds'", region.end, clamp.after.0)
    };
    format!("{col} >= {lower} AND {col} < {upper}", col = clamp.column)
}

impl Region {
    fn predicate(&self, qualifier: Option<&str>, column: &str) -> String {
        let col = match qualifier {
            Some(q) => format!("{q}.{column}"),
            None => column.to_string(),
        };
        format!(
            "{col} >= {start} AND {col} < {end}",
            start = self.start,
            end = self.end
        )
    }
}

/// Recompute-a-region (bottom-right): `DELETE` exactly the write window,
/// `INSERT` its recompute, as one transactional [`StatementGroup`]. The
/// DELETE's range must equal exactly what the INSERT writes
/// (`docs/specs/model_transforms.md` §"Write window = output window"); the
/// INSERT does **not** re-add the predicate — `body` is the caller's
/// already-clamped compiled SELECT (the output clamp is injected upstream,
/// `smelt-runtime/src/transformer.rs`), so re-wrapping it here would be a
/// second, redundant filter the emitter has no business adding.
///
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; the DELETE/INSERT shape is currently dialect-invariant
/// (DuckDB and Spark share the same `DELETE FROM … WHERE …` / `INSERT INTO
/// … <select>` grammar for this family).
pub fn emit_delete_insert(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    let pred = region.predicate(None, partition_col);
    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(format!("DELETE FROM {table} WHERE {pred}")),
            MaintenanceStatement::new(format!("INSERT INTO {table} {body}")),
        ],
        transactional: true,
    }
}

/// Column-scoped re-derivation (bottom-left): the keyed `MERGE` production
/// actually executes for `Technique::ColumnScopedMerge`
/// (`crate::maintenance_driver::execute_column_scoped_merge`/
/// `execute_column_scoped_merge_full` in `smelt-runtime`) —
/// `WHEN MATCHED THEN UPDATE SET *`, `WHEN NOT MATCHED THEN INSERT *`. There
/// is no column-list `SET` variant: DuckDB and Spark both key-match on
/// `unique_key` and update every column from `source_select`'s projection
/// (dialect-invariant text for this family — no branch reads `dialect` yet,
/// kept for signature symmetry with the other emitters in this module).
///
/// **Full-row source-projection contract** (moved from
/// `smelt-backend-duckdb`'s doc comment): `UPDATE SET *` requires
/// `source_select`'s projection to carry every target column — DuckDB errors
/// on a column-count/name mismatch, it does not silently subset by name — so
/// the caller must project the FULL target row, not just the re-derived
/// column group, carrying columns outside that group through unchanged from
/// the existing target state (typically via a join back to the target, or —
/// for the model's own recompute SQL — because the model's SELECT already
/// projects every output column by construction). `SET *` then only changes
/// the group's columns' actual values, satisfying `Technique::
/// ColumnScopedMerge`'s contract without a second, column-list-aware MERGE
/// primitive. This emitter does not (and cannot) verify the projection is
/// complete — a caller that violates the contract fails at the backend, not
/// silently.
///
/// Partition-scoping, when the technique is not the declared full-scan case,
/// is the caller's job — fold it into `source_select` the same way
/// `emit_delete_insert`'s caller folds the output clamp into `body`; the
/// emitter adds no predicate of its own on either the scan or the write
/// target (unlike the old, never-production-matching column-list form this
/// replaces).
pub fn emit_column_scoped_merge(
    table: &str,
    unique_key: &[String],
    source_select: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = source.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {table} AS target USING ({source_select}) AS source ON {on} \
             WHEN MATCHED THEN UPDATE SET * \
             WHEN NOT MATCHED THEN INSERT *"
        ))],
        transactional: false,
    }
}

/// In-place field backfill (top-left with an empty input delta): `UPDATE`
/// the stored region from its own columns; no upstream read at all.
pub fn emit_in_place_update(
    table: &str,
    assignments: &[(String, String)],
    partition_col: &str,
    region: &Region,
) -> Vec<String> {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![format!(
        "UPDATE {table} SET {sets} WHERE {}",
        region.predicate(None, partition_col)
    )]
}

/// Fold-a-delta into keyed end-state (top-left): the combiner-aware `MERGE`
/// production actually executes for `refresh: keyed` (`incremental_models.md`,
/// `crate::cumulative::execute_cumulative_aggregate`'s `WindowedKeyedRule`
/// impl) — combine every aggregator column on matched keys via its own
/// cross-partition combiner, insert unseen keys wholesale.
///
/// `folds` is plain data: `(output_column, rendered_combine_expression)`
/// pairs, e.g. `("event_count", "target.event_count + delta.event_count")`
/// or `("first_seen", "LEAST(target.first_seen, delta.first_seen)")`. The
/// caller renders `CrossPartitionCombiner` (`smelt-planner`) to these
/// expression strings *before* calling this emitter — `smelt-logical` sits
/// below `smelt-planner` in the crate layering
/// (`docs/specs/architecture.md` §"Layered single-ownership") and must
/// never depend on it, so the emitter only assembles plain strings it is
/// handed, never chooses or renders a combiner itself.
///
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; the keyed-fold `MERGE` shape is currently dialect-invariant
/// (no branch reads it yet).
pub fn emit_keyed_fold(
    schema_table: &str,
    key: &[String],
    folds: &[(String, String)],
    delta_select: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    let on = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sets = folds
        .iter()
        .map(|(col, expr)| format!("{col} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {schema_table} AS target USING ({delta_select}) AS delta ON {on} \
             WHEN MATCHED THEN UPDATE SET {sets} \
             WHEN NOT MATCHED THEN INSERT *"
        ))],
        transactional: false,
    }
}

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
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "CREATE TABLE {table} AS {select_sql}"
        ))],
        transactional: false,
    }
}

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_models.md` §"First-run and backfill" —
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
        _ => dt.to_backend_sql(),
    }
}
