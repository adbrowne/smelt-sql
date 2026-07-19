//! DuckDB-specific DDL generation from abstract `SchemaOperation`s.
//!
//! Translates backend-agnostic schema operations into DuckDB SQL statements.
//! DuckDB supports:
//! - Dot-notation for struct field access (`ALTER TABLE t ADD COLUMN col.field TYPE`)
//! - `ALTER COLUMN TYPE ... USING expr` for type rewrites (e.g., `struct_pack(...)`)
//! - `list_transform` for array element transformations

use crate::schema_tracking::{quote_identifier, SchemaOperation};
use smelt_types::DataType;

/// Generate DuckDB-specific DDL statements from a list of `SchemaOperation`s.
///
/// Returns a list of SQL statements to execute in order.
pub fn generate_duckdb_ddl(schema: &str, table: &str, ops: &[SchemaOperation]) -> Vec<String> {
    let qualified = format!("{}.{}", schema, table);
    let mut stmts = Vec::new();

    for op in ops {
        match op {
            SchemaOperation::AddColumn {
                name,
                data_type,
                nullable,
                default_expr,
            } => {
                let type_sql = data_type.to_sql();
                let qname = quote_identifier(name);
                let mut stmt = if *nullable {
                    format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        qualified, qname, type_sql
                    )
                } else {
                    format!(
                        "ALTER TABLE {} ADD COLUMN {} {} NOT NULL",
                        qualified, qname, type_sql
                    )
                };
                if let Some(default) = default_expr {
                    stmt.push_str(&format!(" DEFAULT {}", default));
                }
                stmts.push(stmt);
            }
            SchemaOperation::RemoveColumn { name } => {
                stmts.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified,
                    quote_identifier(name)
                ));
            }
            SchemaOperation::WidenColumnType { name, to, .. } => {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                    qualified,
                    quote_identifier(name),
                    to.to_sql()
                ));
            }
            SchemaOperation::ChangeNullability {
                name,
                to_nullable,
                default_expr,
            } => {
                let qname = quote_identifier(name);
                if *to_nullable {
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                        qualified, qname
                    ));
                } else {
                    // Fill NULLs first, then set NOT NULL
                    if let Some(default) = default_expr {
                        stmts.push(format!(
                            "UPDATE {} SET {} = {} WHERE {} IS NULL",
                            qualified, qname, default, qname
                        ));
                    }
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                        qualified, qname
                    ));
                }
            }
            SchemaOperation::AddStructField {
                column,
                path,
                field_name,
                field_type,
                default_expr: _,
            } => {
                let dot_path = format_dot_path(column, path, Some(field_name));
                stmts.push(format!(
                    "ALTER TABLE {} ADD COLUMN {} {}",
                    qualified,
                    dot_path,
                    field_type.to_sql()
                ));
            }
            SchemaOperation::RemoveStructField {
                column,
                path,
                field_name,
            } => {
                let dot_path = format_dot_path(column, path, Some(field_name));
                stmts.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified, dot_path
                ));
            }
            SchemaOperation::WidenNestedType {
                column,
                path,
                from: _,
                to,
            } => {
                // For DuckDB, nested type widening can use dot-notation ALTER COLUMN TYPE
                // when the path identifies a specific struct field.
                let qcol = quote_identifier(column);
                if path.is_empty() {
                    // Direct column type change — shouldn't normally hit this path
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        qcol,
                        to.to_sql()
                    ));
                } else if path.len() == 1 && path[0] == "value" {
                    // Map value widening — reconstruct the full MAP type.
                    // The key type is unknown here, so use ALTER COLUMN TYPE on the whole column.
                    // DuckDB supports ALTER COLUMN TYPE for map columns.
                    let new_map_type = DataType::Map(
                        Box::new(DataType::Varchar { max_length: None }),
                        Box::new(to.clone()),
                    );
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        qcol,
                        new_map_type.to_sql()
                    ));
                } else {
                    // Struct field widening — use dot-notation: ALTER COLUMN col.path.field TYPE new_type
                    let dot_path = format_dot_path(column, path, None);
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        dot_path,
                        to.to_sql()
                    ));
                }
            }
            SchemaOperation::BackfillColumn { name, expression } => {
                stmts.push(format!(
                    "UPDATE {} SET {} = {}",
                    qualified,
                    quote_identifier(name),
                    expression
                ));
            }
            SchemaOperation::RewriteColumn {
                column,
                target_type,
                using_expr,
            } => {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}",
                    qualified,
                    quote_identifier(column),
                    target_type.to_sql(),
                    using_expr
                ));
            }
        }
    }

    stmts
}

/// Build a `struct_pack(...)` expression that transforms a struct column from
/// `old_type` to `new_type`.
///
/// Handles:
/// - New fields (added with `NULL::TYPE` or default expression)
/// - Widened fields (cast via `::NEW_TYPE`)
/// - Unchanged fields (passed through as-is)
/// - Nested structs (recursive `struct_pack`)
pub fn build_struct_pack_expr(
    column: &str,
    old_type: &DataType,
    new_type: &DataType,
) -> Option<String> {
    match (old_type, new_type) {
        (DataType::Struct(old_fields), DataType::Struct(new_fields)) => {
            let parts = build_struct_pack_parts(column, old_fields, new_fields);
            Some(format!("struct_pack({})", parts.join(", ")))
        }
        _ => None,
    }
}

/// Build the individual field assignments for a struct_pack expression.
fn build_struct_pack_parts(
    column: &str,
    old_fields: &[(String, DataType)],
    new_fields: &[(String, DataType)],
) -> Vec<String> {
    let mut parts = Vec::new();

    for (new_name, new_dt) in new_fields {
        if let Some((_old_name, old_dt)) = old_fields.iter().find(|(n, _)| n == new_name) {
            // Field exists in old type
            let field_ref = format!("{}.{}", column, quote_identifier(new_name));
            if old_dt == new_dt {
                // Unchanged — pass through
                parts.push(format!("{} := {}", new_name, field_ref));
            } else {
                // Type changed — check if it's a nested struct needing recursive struct_pack
                match (old_dt, new_dt) {
                    (DataType::Struct(old_inner), DataType::Struct(new_inner)) => {
                        let inner_expr = build_struct_pack_inner(&field_ref, old_inner, new_inner);
                        parts.push(format!("{} := {}", new_name, inner_expr));
                    }
                    _ => {
                        // Simple type cast
                        parts.push(format!(
                            "{} := {}::{}",
                            new_name,
                            field_ref,
                            new_dt.to_sql()
                        ));
                    }
                }
            }
        } else {
            // New field — use NULL cast to correct type
            parts.push(format!("{} := NULL::{}", new_name, new_dt.to_sql()));
        }
    }

    parts
}

/// Recursively build a nested struct_pack expression for inner structs.
fn build_struct_pack_inner(
    field_ref: &str,
    old_fields: &[(String, DataType)],
    new_fields: &[(String, DataType)],
) -> String {
    let parts = build_struct_pack_parts(field_ref, old_fields, new_fields);
    format!("struct_pack({})", parts.join(", "))
}

/// Build a `list_transform(column, x -> struct_pack(...))` expression for
/// array-of-struct widening in DuckDB.
///
/// Returns `None` if either type is not `Array(Struct(...))`.
pub fn build_list_transform_expr(
    column: &str,
    old_type: &DataType,
    new_type: &DataType,
) -> Option<String> {
    match (old_type, new_type) {
        (DataType::Array(old_elem), DataType::Array(new_elem)) => {
            match (old_elem.as_ref(), new_elem.as_ref()) {
                (DataType::Struct(old_fields), DataType::Struct(new_fields)) => {
                    let parts = build_struct_pack_parts("x", old_fields, new_fields);
                    Some(format!(
                        "list_transform({}, x -> struct_pack({}))",
                        column,
                        parts.join(", ")
                    ))
                }
                _ => {
                    // Simple array element cast — no list_transform needed,
                    // DuckDB handles this with ALTER COLUMN TYPE directly
                    None
                }
            }
        }
        _ => None,
    }
}

/// Build a USING expression for column type changes.
///
/// Determines whether to use `struct_pack`, `list_transform`, or a simple cast
/// based on the old and new types.
pub fn build_using_expr(column: &str, old_type: &DataType, new_type: &DataType) -> Option<String> {
    // Try struct_pack first (struct columns)
    if let Some(expr) = build_struct_pack_expr(column, old_type, new_type) {
        return Some(expr);
    }
    // Try list_transform (array-of-struct columns)
    if let Some(expr) = build_list_transform_expr(column, old_type, new_type) {
        return Some(expr);
    }
    None
}

/// Format a dot-separated path for DuckDB struct field access.
/// e.g., `format_dot_path("meta", &["inner"], Some("field"))` → `"meta.inner.field"`
fn format_dot_path(column: &str, path: &[String], leaf: Option<&str>) -> String {
    let mut parts = vec![quote_identifier(column)];
    parts.extend(path.iter().map(|p| quote_identifier(p)));
    if let Some(l) = leaf {
        parts.push(quote_identifier(l));
    }
    parts.join(".")
}

// ── Warehouse-resident per-delta reconciliation ledger (MP12) ──────────────
//
// `docs/specs/incremental_models.md` §"The reconciliation ledger": an
// **additive**-graded group's storage must record delta identities, not just
// a frontier watermark, because re-folding one double-counts. The ledger
// table generated here is that storage for the keyed `merge_into` fold path
// (`crates/smelt-runtime/src/maintenance_driver.rs`): its `PRIMARY KEY` over
// `(model_name, grp, input_name, delta_id)` **is** the never-fold-twice key,
// so the caller's paired `INSERT` (this table) + fold/create action (the
// model's own write) can run as one backend transaction and let the
// constraint itself refuse a repeat — no separate, racy existence check is
// load-bearing for correctness (a `SELECT`-based existence check is still
// generated below for backends that cannot wrap both statements in one
// transaction; see `smelt_backend::Backend::fold_ledger_delta`'s default).
//
// Idempotent-graded groups never call any of these — a warehouse ledger
// table only exists for a project once an additive-fold cell has actually
// folded (`docs/specs/incremental_models.md` §Constraints; review checklist
// "no warehouse tables for idempotent-only plans").

/// Table name for the warehouse-resident per-delta reconciliation ledger.
pub const LEDGER_TABLE_NAME: &str = "_smelt_ledger";

/// DDL creating the ledger table if it does not already exist. Idempotent —
/// safe to run before every fold.
pub fn generate_ledger_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {}.{} (\
         model_name VARCHAR NOT NULL, \
         grp VARCHAR NOT NULL, \
         input_name VARCHAR NOT NULL, \
         delta_id VARCHAR NOT NULL, \
         region_start VARCHAR NOT NULL, \
         region_end VARCHAR NOT NULL, \
         PRIMARY KEY (model_name, grp, input_name, delta_id))",
        quote_identifier(schema),
        LEDGER_TABLE_NAME,
    )
}

/// `INSERT` recording one delta identity as folded for `(model, group,
/// input)`. Violates the table's `PRIMARY KEY` — the never-fold-twice key —
/// iff `delta_id` is already reflected.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_insert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    format!(
        "INSERT INTO {}.{} (model_name, grp, input_name, delta_id, region_start, region_end) \
         VALUES ('{}', '{}', '{}', '{}', '{}', '{}')",
        quote_identifier(schema),
        LEDGER_TABLE_NAME,
        escape_sql_literal(model),
        escape_sql_literal(group),
        escape_sql_literal(input),
        escape_sql_literal(delta_id),
        escape_sql_literal(region_start),
        escape_sql_literal(region_end),
    )
}

/// Existence check for `(model, group, input, delta_id)` — the best-effort
/// fallback `Backend::fold_ledger_delta` default uses on a backend that
/// cannot wrap the insert and the fold action in one native transaction.
pub fn generate_ledger_exists_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
) -> String {
    format!(
        "SELECT 1 FROM {}.{} WHERE model_name = '{}' AND grp = '{}' AND input_name = '{}' \
         AND delta_id = '{}' LIMIT 1",
        quote_identifier(schema),
        LEDGER_TABLE_NAME,
        escape_sql_literal(model),
        escape_sql_literal(group),
        escape_sql_literal(input),
        escape_sql_literal(delta_id),
    )
}

fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

// ── Warehouse-resident observed output delta (T5) ───────────────────────
//
// `docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas
// on model edges": a conditional write (`WriteSuppression::Suppressed`)
// already computes the changed-row set it actually touched; this table
// records it — key-level v1 (the model's `unique_key` values plus the
// touched partition value, comparable columns only). It is warehouse-
// resident, alongside the reconciliation ledger above, and — per the D1
// spec ruling — in the SAME excluded "bookkeeping" class as that ledger:
// this is smelt-state storage for a run's own byproduct, not a maintenance
// statement a run's *write* executes, so it sits outside
// `smelt_logical::maintenance::emit`'s single-owner rule
// (`crates/smelt-runtime/tests/statement_parity.rs`'s structural
// no-authoring gate does not scan `smelt-state`, matching the ledger's own
// precedent).
//
// One row per `(model_name, window_start, window_end)` — a re-run of the
// same window is an idempotent REPLACE (`ON CONFLICT ... DO UPDATE`), never
// a duplicate: this is a **recorded snapshot of the last run's delta for
// that window**, not an append-only log. `changed_keys`/`partitions` are
// `VARCHAR[]` (never `NULL` — an empty array is a real, present-and-empty
// delta; a `NULL` never appears because the upsert always coalesces to
// `[]`). Absent a row for a window is the "no delta was ever recorded"
// case a consumer must not conflate with an empty delta
// (`incremental_models.md` §"The graph layer" — "Empty and absent are
// distinct").

/// Table name for the warehouse-resident observed-output-delta record.
pub const OBSERVED_DELTA_TABLE_NAME: &str = "_smelt_observed_delta";

/// DDL creating the observed-delta table if it does not already exist.
/// Idempotent — safe to run before every conditional write.
pub fn generate_observed_delta_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {}.{} (\
         model_name VARCHAR NOT NULL, \
         window_start VARCHAR NOT NULL, \
         window_end VARCHAR NOT NULL, \
         changed_keys VARCHAR[] NOT NULL, \
         partitions VARCHAR[] NOT NULL, \
         PRIMARY KEY (model_name, window_start, window_end))",
        quote_identifier(schema),
        OBSERVED_DELTA_TABLE_NAME,
    )
}

/// Upsert one `(model, run window)`'s observed delta: `changed_keys_query`
/// is the caller's already-built SELECT of every changed row's `unique_key`
/// value(s), collapsed to one `VARCHAR` column named `delta_key`;
/// `partition_expr` is `Some("<column-or-expr>")` naming the touched-
/// partition projection carried by that same query as a `delta_partition`
/// column (only meaningful for a composed, locality-admitted model), or
/// `None` when the write has no partition axis (a bare keyed model's
/// dimension MERGE) — in that case `partitions` is always recorded empty.
/// `ARRAY_AGG` over zero rows is `NULL` in DuckDB; `COALESCE` folds that
/// back to `[]` so a fully-suppressed run (nothing changed) still inserts a
/// row — present-and-empty, not absent. Re-running the same window replaces
/// via `ON CONFLICT ... DO UPDATE`, never duplicating
/// (`PRIMARY KEY (model_name, window_start, window_end)` is the same
/// window's own replace key).
pub fn generate_observed_delta_upsert_sql(
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
    changed_keys_query: &str,
) -> String {
    format!(
        "INSERT INTO {schema}.{table} (model_name, window_start, window_end, changed_keys, partitions) \
         SELECT '{model}', '{window_start}', '{window_end}', \
         COALESCE(ARRAY_AGG(DISTINCT __smelt_delta.delta_key) FILTER (WHERE __smelt_delta.delta_key IS NOT NULL), []::VARCHAR[]), \
         COALESCE(ARRAY_AGG(DISTINCT __smelt_delta.delta_partition) FILTER (WHERE __smelt_delta.delta_partition IS NOT NULL), []::VARCHAR[]) \
         FROM ({changed_keys_query}) AS __smelt_delta \
         ON CONFLICT (model_name, window_start, window_end) DO UPDATE SET \
         changed_keys = excluded.changed_keys, partitions = excluded.partitions",
        schema = quote_identifier(schema),
        table = OBSERVED_DELTA_TABLE_NAME,
        model = escape_sql_literal(model),
        window_start = escape_sql_literal(window_start),
        window_end = escape_sql_literal(window_end),
        changed_keys_query = changed_keys_query,
    )
}

/// Read-side counterpart to [`generate_observed_delta_upsert_sql`]: the SQL
/// to fetch one `(model, window)`'s recorded row back
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase D3
/// — "Key→partition projection of observed deltas"). Pure string generation
/// only, matching this module's own convention — the backend executes it
/// and decodes the two `VARCHAR[]` columns into an [`ObservedDelta`]; a
/// caller finding **no row** for the `(model, window)` key is the "never
/// recorded" case (`incremental_models.md` §"The graph layer": "Empty and
/// absent are distinct" — widen-never-narrow falls back to the written
/// window), which this SQL alone cannot distinguish from a genuine
/// zero-row query result — the caller's row-count check is what tells them
/// apart, exactly as the DDL/DML tests above already exercise via
/// `execute_sql(...).num_rows()`.
pub fn generate_observed_delta_select_sql(
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> String {
    format!(
        "SELECT changed_keys, partitions FROM {schema}.{table} \
         WHERE model_name = '{model}' AND window_start = '{window_start}' AND window_end = '{window_end}'",
        schema = quote_identifier(schema),
        table = OBSERVED_DELTA_TABLE_NAME,
        model = escape_sql_literal(model),
        window_start = escape_sql_literal(window_start),
        window_end = escape_sql_literal(window_end),
    )
}

/// One `(model, window)` row read back from the observed-delta table — the
/// backend's decoded counterpart of the two `VARCHAR[]` columns
/// [`generate_observed_delta_select_sql`] projects. `Option<ObservedDelta>`
/// at the call site (not this type) carries the empty-vs-absent distinction:
/// `None` means no row was found for that `(model, window)` (never
/// recorded); `Some` with both vectors empty means the row exists and
/// records a fully-suppressed run (present-and-empty). This type itself
/// only ever represents a **found** row, so its own `is_empty` reports
/// whether that found row's delta was empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedDelta {
    /// The model's `unique_key` values that changed in this window.
    pub changed_keys: Vec<String>,
    /// The touched partition values (the model's `timeseries.partition_column`
    /// projection), empty when the model has no partition axis.
    pub partitions: Vec<String>,
}

impl ObservedDelta {
    /// Whether this **found** row recorded a fully-suppressed run (nothing
    /// changed) — distinct from "no row was found at all"
    /// (`Option<ObservedDelta>::None` at the call site).
    pub fn is_empty(&self) -> bool {
        self.changed_keys.is_empty() && self.partitions.is_empty()
    }
}

// ── Warehouse-resident row-content fingerprint sidecar (F3,
// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F3;
// `docs/specs/sources.md` §"The fingerprint sidecar") ───────────────────
//
// Synthesizes a change feed for a `mutable_snapshot` external source with
// no native change feed: a row-content digest last observed for each
// source key, that a consuming run diffs the source's CURRENT content
// against to derive an exact changed-key set instead of treating every
// re-scan as a whole-table delta. This table's own DDL/DML is warehouse-
// resident bookkeeping, in the SAME excluded class as the reconciliation
// ledger and the observed-output-delta table above (`incremental_models.md`
// §"Statement emission (single owner)"'s third exclusion) — DuckDB-scoped
// today, matching both those precedents; no Spark DDL exists here, and a
// non-DuckDB target fails loud at the call site
// (`crates/smelt-runtime/src/maintenance_driver.rs`) rather than being
// handed this DuckDB-flavored SQL.
//
// A row is namespaced by `(source_address, projection_identity,
// source_key)` — projection identity, not consumer identity: a canonical
// identifier of the P4 fingerprint projection's column set
// (`smelt_logical::analysis::fingerprint::projection_identity`). The
// synthesized DIFF query that compares current source content against this
// table is emitter-authored
// (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`) —
// unlike this table's own storage DDL/DML, the diff is a maintenance-
// relevant derived comparison, not a record of smelt's own run history.

/// Table name for the warehouse-resident row-content fingerprint sidecar.
pub const FINGERPRINT_SIDECAR_TABLE_NAME: &str = "_smelt_fingerprint_sidecar";

/// DDL creating the fingerprint sidecar table if it does not already
/// exist. Idempotent — safe to run before every diff/refresh.
pub fn generate_fingerprint_sidecar_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {}.{} (\
         source_address VARCHAR NOT NULL, \
         projection_identity VARCHAR NOT NULL, \
         source_key VARCHAR NOT NULL, \
         digest VARCHAR NOT NULL, \
         PRIMARY KEY (source_address, projection_identity, source_key))",
        quote_identifier(schema),
        FINGERPRINT_SIDECAR_TABLE_NAME,
    )
}

/// Upsert every currently-observed `(key, digest)` pair
/// `digest_select_sql` projects
/// (`smelt_logical::maintenance::emit::emit_fingerprint_digest_select`'s
/// output — `delta_key`/`delta_digest` columns) into the sidecar partition
/// named by `(source_address, projection_identity)`. Idempotent: an
/// unchanged key's digest is overwritten with the identical value it
/// already held (`ON CONFLICT ... DO UPDATE`), never duplicated — the same
/// "re-running a window is a REPLACE, never a duplicate" shape
/// [`generate_observed_delta_upsert_sql`] uses above.
///
/// A source key absent from the sidecar before this call is, by
/// construction, new — this upsert is what "populates the sidecar as a
/// byproduct" of a first run against an unpopulated one
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "First run and
/// `--full-refresh`").
pub fn generate_fingerprint_sidecar_refresh_sql(
    schema: &str,
    source_address: &str,
    projection_identity: &str,
    digest_select_sql: &str,
) -> String {
    format!(
        "INSERT INTO {schema}.{table} (source_address, projection_identity, source_key, digest) \
         SELECT '{source_address}', '{projection_identity}', __smelt_digest.delta_key, \
         __smelt_digest.delta_digest FROM ({digest_select_sql}) AS __smelt_digest \
         ON CONFLICT (source_address, projection_identity, source_key) DO UPDATE SET \
         digest = excluded.digest",
        schema = quote_identifier(schema),
        table = FINGERPRINT_SIDECAR_TABLE_NAME,
        source_address = escape_sql_literal(source_address),
        projection_identity = escape_sql_literal(projection_identity),
        digest_select_sql = digest_select_sql,
    )
}

/// GC a sidecar partition's keys that no longer appear in the source
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "GC": "A key's
/// disappearance from the source is itself a change ... and drops its
/// sidecar row"). Must run against the SAME `digest_select_sql` the paired
/// diff query read, in the same transaction as that diff's consuming write
/// (`Backend::execute_write_and_refresh_fingerprint_sidecar`), so a key
/// deleted between the diff and this refresh cannot be silently left
/// behind or double-counted next run.
pub fn generate_fingerprint_sidecar_gc_sql(
    schema: &str,
    source_address: &str,
    projection_identity: &str,
    digest_select_sql: &str,
) -> String {
    format!(
        "DELETE FROM {schema}.{table} WHERE source_address = '{source_address}' AND \
         projection_identity = '{projection_identity}' AND source_key NOT IN \
         (SELECT __smelt_digest.delta_key FROM ({digest_select_sql}) AS __smelt_digest)",
        schema = quote_identifier(schema),
        table = FINGERPRINT_SIDECAR_TABLE_NAME,
        source_address = escape_sql_literal(source_address),
        projection_identity = escape_sql_literal(projection_identity),
        digest_select_sql = digest_select_sql,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::DataType;

    // ── generate_duckdb_ddl tests ──────────────────────────────────────

    #[test]
    fn test_add_column_nullable() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "status".into(),
            data_type: DataType::Varchar { max_length: None },
            nullable: true,
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t ADD COLUMN status VARCHAR"]);
    }

    #[test]
    fn test_add_column_not_null_with_default() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "count".into(),
            data_type: DataType::Integer,
            nullable: false,
            default_expr: Some("0".into()),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ADD COLUMN count INTEGER NOT NULL DEFAULT 0"]
        );
    }

    #[test]
    fn test_remove_column() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "old_col".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t DROP COLUMN old_col"]);
    }

    #[test]
    fn test_widen_column_type() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN amount TYPE BIGINT"]
        );
    }

    #[test]
    fn test_change_nullability_to_nullable() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: true,
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN status DROP NOT NULL"]
        );
    }

    #[test]
    fn test_change_nullability_to_not_null() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: false,
            default_expr: Some("'unknown'".into()),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec![
                "UPDATE main.t SET status = 'unknown' WHERE status IS NULL",
                "ALTER TABLE main.t ALTER COLUMN status SET NOT NULL",
            ]
        );
    }

    #[test]
    fn test_add_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t ADD COLUMN meta.b VARCHAR"]);
    }

    #[test]
    fn test_add_nested_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "data".into(),
            path: vec!["inner".into()],
            field_name: "y".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ADD COLUMN data.\"inner\".y VARCHAR"]
        );
    }

    #[test]
    fn test_remove_struct_field() {
        let ops = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "old_field".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t DROP COLUMN meta.old_field"]);
    }

    #[test]
    fn test_rewrite_column_struct_pack() {
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![
                ("a".into(), DataType::BigInt),
                ("b".into(), DataType::Varchar { max_length: None }),
            ]),
            using_expr: "struct_pack(a := meta.a::BIGINT, b := meta.b)".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec![
                "ALTER TABLE main.t ALTER COLUMN meta TYPE STRUCT(a BIGINT, b VARCHAR) USING struct_pack(a := meta.a::BIGINT, b := meta.b)"
            ]
        );
    }

    #[test]
    fn test_widen_array_column() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "scores".into(),
            from: DataType::Array(Box::new(DataType::Integer)),
            to: DataType::Array(Box::new(DataType::BigInt)),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN scores TYPE BIGINT[]"]
        );
    }

    #[test]
    fn test_backfill_column() {
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "status".into(),
            expression: "CASE WHEN active THEN 'active' ELSE 'inactive' END".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["UPDATE main.t SET status = CASE WHEN active THEN 'active' ELSE 'inactive' END"]
        );
    }

    #[test]
    fn test_multiple_operations() {
        let ops = vec![
            SchemaOperation::AddColumn {
                name: "new_col".into(),
                data_type: DataType::Integer,
                nullable: true,
                default_expr: None,
            },
            SchemaOperation::RemoveColumn {
                name: "old_col".into(),
            },
            SchemaOperation::WidenColumnType {
                name: "amount".into(),
                from: DataType::Integer,
                to: DataType::BigInt,
            },
        ];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl.len(), 3);
        assert_eq!(ddl[0], "ALTER TABLE main.t ADD COLUMN new_col INTEGER");
        assert_eq!(ddl[1], "ALTER TABLE main.t DROP COLUMN old_col");
        assert_eq!(ddl[2], "ALTER TABLE main.t ALTER COLUMN amount TYPE BIGINT");
    }

    // ── build_struct_pack_expr tests ───────────────────────────────────

    #[test]
    fn test_struct_pack_field_widening() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(expr, "struct_pack(a := meta.a::BIGINT, b := meta.b)");
    }

    #[test]
    fn test_struct_pack_new_field() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
            ("c".into(), DataType::Boolean),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(a := meta.a, b := meta.b, c := NULL::BOOLEAN)"
        );
    }

    #[test]
    fn test_struct_pack_widen_and_add() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
            ("c".into(), DataType::Boolean),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(a := meta.a::BIGINT, b := meta.b, c := NULL::BOOLEAN)"
        );
    }

    #[test]
    fn test_struct_pack_nested_struct() {
        let old = DataType::Struct(vec![
            (
                "inner".into(),
                DataType::Struct(vec![("x".into(), DataType::Integer)]),
            ),
            ("b".into(), DataType::BigInt),
        ]);
        let new = DataType::Struct(vec![
            (
                "inner".into(),
                DataType::Struct(vec![
                    ("x".into(), DataType::BigInt),
                    ("y".into(), DataType::Varchar { max_length: None }),
                ]),
            ),
            ("b".into(), DataType::BigInt),
        ]);
        let expr = build_struct_pack_expr("data", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(inner := struct_pack(x := data.\"inner\".x::BIGINT, y := NULL::VARCHAR), b := data.b)"
        );
    }

    #[test]
    fn test_struct_pack_non_struct_returns_none() {
        let old = DataType::Integer;
        let new = DataType::BigInt;
        assert!(build_struct_pack_expr("col", &old, &new).is_none());
    }

    #[test]
    fn test_widen_nested_type_struct_field() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN meta.a TYPE BIGINT"]
        );
    }

    #[test]
    fn test_widen_nested_type_map_value() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "m".into(),
            path: vec!["value".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN m TYPE MAP(VARCHAR, BIGINT)"]
        );
    }

    #[test]
    fn test_widen_nested_type_deeply_nested() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "data".into(),
            path: vec!["inner".into(), "x".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN data.\"inner\".x TYPE BIGINT"]
        );
    }

    // ── build_list_transform_expr tests ────────────────────────────────

    #[test]
    fn test_list_transform_array_of_struct_widen() {
        let old = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::Integer,
        )])));
        let new = DataType::Array(Box::new(DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
        ])));
        let expr = build_list_transform_expr("items", &old, &new).unwrap();
        assert_eq!(
            expr,
            "list_transform(items, x -> struct_pack(a := x.a::BIGINT, b := NULL::VARCHAR))"
        );
    }

    #[test]
    fn test_list_transform_simple_array_returns_none() {
        let old = DataType::Array(Box::new(DataType::Integer));
        let new = DataType::Array(Box::new(DataType::BigInt));
        assert!(build_list_transform_expr("scores", &old, &new).is_none());
    }

    #[test]
    fn test_list_transform_non_array_returns_none() {
        let old = DataType::Integer;
        let new = DataType::BigInt;
        assert!(build_list_transform_expr("col", &old, &new).is_none());
    }

    // ── build_using_expr tests ─────────────────────────────────────────

    #[test]
    fn test_using_expr_struct() {
        let old = DataType::Struct(vec![("a".into(), DataType::Integer)]);
        let new = DataType::Struct(vec![("a".into(), DataType::BigInt)]);
        let expr = build_using_expr("meta", &old, &new).unwrap();
        assert!(expr.starts_with("struct_pack("));
    }

    #[test]
    fn test_using_expr_array_of_struct() {
        let old = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::Integer,
        )])));
        let new = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::BigInt,
        )])));
        let expr = build_using_expr("items", &old, &new).unwrap();
        assert!(expr.starts_with("list_transform("));
    }

    #[test]
    fn test_using_expr_simple_types_returns_none() {
        assert!(build_using_expr("col", &DataType::Integer, &DataType::BigInt).is_none());
    }

    // ── struct_pack additional tests ───────────────────────────────────

    #[test]
    fn test_struct_pack_all_unchanged() {
        let dt = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let expr = build_struct_pack_expr("meta", &dt, &dt).unwrap();
        assert_eq!(expr, "struct_pack(a := meta.a, b := meta.b)");
    }

    // ── quoting tests ────────────────────────────────────────────────────

    #[test]
    fn test_add_column_with_keyword_name() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "select".to_string(),
            data_type: DataType::Integer,
            nullable: true,
            default_expr: None,
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"select\""),
            "SQL keyword column name should be quoted, got: {}",
            stmts[0]
        );
    }

    #[test]
    fn test_add_struct_field_with_space_in_name() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".to_string(),
            path: vec![],
            field_name: "first name".to_string(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"first name\""),
            "Field name with space should be quoted, got: {}",
            stmts[0]
        );
    }

    #[test]
    fn test_remove_column_with_keyword_name() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "order".to_string(),
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"order\""),
            "SQL keyword column name should be quoted, got: {}",
            stmts[0]
        );
    }

    // ── warehouse-resident per-delta ledger DDL/DML (MP12) ─────────────

    #[test]
    fn ledger_table_ddl_is_idempotent_and_keys_never_fold_twice() {
        let ddl = generate_ledger_table_ddl("main");
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS main._smelt_ledger"));
        assert!(ddl.contains("PRIMARY KEY (model_name, grp, input_name, delta_id)"));
    }

    #[test]
    fn ledger_insert_sql_carries_every_field() {
        let sql = generate_ledger_insert_sql(
            "main",
            "device_stats",
            "{*}",
            "smelt.events",
            "2026-01-01",
            "2026-01-01",
            "2026-01-02",
        );
        assert!(sql.contains("INSERT INTO main._smelt_ledger"));
        assert!(sql.contains("'device_stats'"));
        assert!(sql.contains("'{*}'"));
        assert!(sql.contains("'smelt.events'"));
        assert!(sql.contains("'2026-01-01'"));
        assert!(sql.contains("'2026-01-02'"));
    }

    #[test]
    fn ledger_sql_escapes_single_quotes_in_values() {
        let sql = generate_ledger_insert_sql(
            "main",
            "model's_name",
            "{*}",
            "smelt.events",
            "d1",
            "2026-01-01",
            "2026-01-02",
        );
        assert!(sql.contains("'model''s_name'"));
    }

    #[test]
    fn ledger_exists_sql_filters_on_every_key_field() {
        let sql =
            generate_ledger_exists_sql("main", "device_stats", "{*}", "smelt.events", "2026-01-01");
        assert!(sql.contains("model_name = 'device_stats'"));
        assert!(sql.contains("grp = '{*}'"));
        assert!(sql.contains("input_name = 'smelt.events'"));
        assert!(sql.contains("delta_id = '2026-01-01'"));
    }

    // ── warehouse-resident observed-output-delta DDL/DML (T5) ──────────

    #[test]
    fn observed_delta_table_ddl_is_idempotent_and_windows_never_duplicate() {
        let ddl = generate_observed_delta_table_ddl("main");
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS main._smelt_observed_delta"));
        assert!(ddl.contains("changed_keys VARCHAR[] NOT NULL"));
        assert!(ddl.contains("partitions VARCHAR[] NOT NULL"));
        assert!(ddl.contains("PRIMARY KEY (model_name, window_start, window_end)"));
    }

    #[test]
    fn observed_delta_upsert_replaces_on_conflict_never_duplicates() {
        let sql = generate_observed_delta_upsert_sql(
            "main",
            "dim_users",
            "2026-01-01",
            "2026-01-02",
            "SELECT id AS delta_key, NULL AS delta_partition FROM changed",
        );
        assert!(sql.contains("INSERT INTO main._smelt_observed_delta"));
        assert!(sql.contains("'dim_users'"));
        assert!(sql.contains("'2026-01-01'"));
        assert!(sql.contains("'2026-01-02'"));
        assert!(sql.contains("ON CONFLICT (model_name, window_start, window_end) DO UPDATE SET"));
        assert!(sql.contains("SELECT id AS delta_key, NULL AS delta_partition FROM changed"));
    }

    #[test]
    fn observed_delta_upsert_coalesces_empty_delta_to_present_empty_arrays() {
        // A fully-suppressed run's changed-keys query returns zero rows;
        // ARRAY_AGG over zero rows is NULL in DuckDB, so the upsert must
        // COALESCE to `[]` — the row still gets inserted (present), just
        // with empty arrays, never a NULL array (which would collapse the
        // empty/absent distinction the spec requires).
        let sql = generate_observed_delta_upsert_sql(
            "main",
            "dim_users",
            "2026-01-01",
            "2026-01-02",
            "SELECT id AS delta_key, NULL AS delta_partition FROM changed WHERE FALSE",
        );
        assert!(sql.contains("COALESCE(ARRAY_AGG(DISTINCT __smelt_delta.delta_key)"));
        assert!(sql.contains("[]::VARCHAR[]"));
    }

    #[test]
    fn observed_delta_upsert_escapes_single_quotes_in_model_name() {
        let sql = generate_observed_delta_upsert_sql(
            "main",
            "model's_name",
            "2026-01-01",
            "2026-01-02",
            "SELECT id AS delta_key, NULL AS delta_partition FROM changed",
        );
        assert!(sql.contains("'model''s_name'"));
    }

    // ── observed-delta read API (D3) ────────────────────────────────────

    #[test]
    fn observed_delta_select_sql_filters_on_every_key_field() {
        let sql = generate_observed_delta_select_sql(
            "main",
            "silver.events_deduped",
            "2026-01-01",
            "2026-01-02",
        );
        assert!(sql.contains("SELECT changed_keys, partitions FROM main._smelt_observed_delta"));
        assert!(sql.contains("model_name = 'silver.events_deduped'"));
        assert!(sql.contains("window_start = '2026-01-01'"));
        assert!(sql.contains("window_end = '2026-01-02'"));
    }

    #[test]
    fn observed_delta_select_sql_escapes_single_quotes_in_model_name() {
        let sql =
            generate_observed_delta_select_sql("main", "model's_name", "2026-01-01", "2026-01-02");
        assert!(sql.contains("model_name = 'model''s_name'"));
    }

    #[test]
    fn observed_delta_is_empty_distinguishes_found_empty_from_nonempty() {
        assert!(ObservedDelta::default().is_empty());
        assert!(ObservedDelta {
            changed_keys: vec![],
            partitions: vec![],
        }
        .is_empty());
        assert!(!ObservedDelta {
            changed_keys: vec!["k1".to_string()],
            partitions: vec!["2026-01-01".to_string()],
        }
        .is_empty());
    }

    // ── warehouse-resident row-content fingerprint sidecar (F3) ─────────

    #[test]
    fn fingerprint_sidecar_table_ddl_is_idempotent_and_keyed_by_source_key() {
        let ddl = generate_fingerprint_sidecar_table_ddl("main");
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS main._smelt_fingerprint_sidecar"));
        assert!(ddl.contains("source_address VARCHAR NOT NULL"));
        assert!(ddl.contains("projection_identity VARCHAR NOT NULL"));
        assert!(ddl.contains("source_key VARCHAR NOT NULL"));
        assert!(ddl.contains("digest VARCHAR NOT NULL"));
        assert!(ddl.contains("PRIMARY KEY (source_address, projection_identity, source_key)"));
    }

    #[test]
    fn fingerprint_sidecar_refresh_sql_upserts_on_conflict() {
        let sql = generate_fingerprint_sidecar_refresh_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name,tier",
            "SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users",
        );
        assert!(sql.contains("INSERT INTO main._smelt_fingerprint_sidecar"));
        assert!(sql.contains("'smelt.sources.dim_users'"));
        assert!(sql.contains("'cols:name,tier'"));
        assert!(sql.contains(
            "ON CONFLICT (source_address, projection_identity, source_key) DO UPDATE SET"
        ));
        assert!(sql.contains("SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users"));
    }

    #[test]
    fn fingerprint_sidecar_refresh_sql_escapes_single_quotes() {
        let sql = generate_fingerprint_sidecar_refresh_sql(
            "main",
            "smelt.sources.dim's_users",
            "cols:name",
            "SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users",
        );
        assert!(sql.contains("'smelt.sources.dim''s_users'"));
    }

    #[test]
    fn fingerprint_sidecar_gc_sql_deletes_orphaned_keys() {
        let sql = generate_fingerprint_sidecar_gc_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name,tier",
            "SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users",
        );
        assert!(sql.contains("DELETE FROM main._smelt_fingerprint_sidecar"));
        assert!(sql.contains("source_address = 'smelt.sources.dim_users'"));
        assert!(sql.contains("projection_identity = 'cols:name,tier'"));
        assert!(sql.contains("source_key NOT IN"));
        assert!(sql.contains("SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users"));
    }

    #[test]
    fn fingerprint_sidecar_gc_sql_escapes_single_quotes() {
        let sql = generate_fingerprint_sidecar_gc_sql(
            "main",
            "smelt.sources.dim's_users",
            "cols:name",
            "SELECT id AS delta_key, 'abc' AS delta_digest FROM raw.dim_users",
        );
        assert!(sql.contains("source_address = 'smelt.sources.dim''s_users'"));
    }
}
