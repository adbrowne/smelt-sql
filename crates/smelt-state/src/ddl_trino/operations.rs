//! Per-`SchemaOperation` classification into Trino DDL statements or a
//! named refusal.
//!
//! Every rule is a fact measured against a live coordinator by
//! `scripts/trino-probe-ddl.sh` — see the crate module header.

use super::types::{dotted_path, quote_ident, trino_type_sql};
use crate::schema_tracking::SchemaOperation;
use smelt_dialect::BackendCapabilities;
use smelt_types::DataType;

/// Render one `SchemaOperation` as Trino DDL/DML statements, or `Err` with
/// the reason it cannot be expressed — the caller turns that into
/// `TrinoMigration::FullRefreshRequired`.
///
/// `caps` gates the struct/array/column-mapping forms this module measured
/// against `BackendCapabilities::trino_iceberg()`; a capability set that
/// turns one of them off degrades to the same refusal Spark's Parquet path
/// uses, rather than sending DDL the catalog was never measured to accept.
pub(super) fn ddl_for_operation(
    qualified: &str,
    op: &SchemaOperation,
    caps: &BackendCapabilities,
) -> Result<Vec<String>, String> {
    match op {
        SchemaOperation::AddColumn {
            name,
            data_type,
            nullable,
            default_expr,
        } => {
            if !*nullable {
                return Err(format!(
                    "Trino/Iceberg cannot add the NOT NULL column '{}' to an existing table \
                     (`This connector does not support adding not null columns`)",
                    name
                ));
            }
            let type_sql =
                trino_type_sql(data_type).map_err(|why| format!("column '{}': {}", name, why))?;
            let qname = quote_ident(name);
            let mut stmts = vec![format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                qualified, qname, type_sql
            )];
            if let Some(default) = default_expr {
                // Iceberg accepts no persistent DEFAULT at all (measured:
                // `Default column values are not supported for Iceberg
                // table format version < 3`), so a declared `default:`
                // only ever backfills the rows already in the table —
                // future rows are filled by the model's own recompute, not
                // by the table.
                stmts.push(format!(
                    "UPDATE {} SET {} = {} WHERE {} IS NULL",
                    qualified, qname, default, qname
                ));
            }
            Ok(stmts)
        }
        SchemaOperation::RemoveColumn { name } => Ok(vec![format!(
            "ALTER TABLE {} DROP COLUMN {}",
            qualified,
            quote_ident(name)
        )]),
        SchemaOperation::WidenColumnType { name, to, .. } => {
            let type_sql =
                trino_type_sql(to).map_err(|why| format!("column '{}': {}", name, why))?;
            Ok(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DATA TYPE {}",
                qualified,
                quote_ident(name),
                type_sql
            )])
        }
        SchemaOperation::ChangeNullability {
            name,
            to_nullable,
            default_expr,
        } => {
            if !*to_nullable {
                return Err(format!(
                    "Trino/Iceberg has no ALTER COLUMN … SET NOT NULL (the parser rejects it: \
                     `Expecting: 'DATA', 'DEFAULT'`), so column '{}' cannot be tightened to \
                     NOT NULL in place",
                    name
                ));
            }
            let qname = quote_ident(name);
            let mut stmts = Vec::new();
            if let Some(default) = default_expr {
                stmts.push(format!(
                    "UPDATE {} SET {} = {} WHERE {} IS NULL",
                    qualified, qname, default, qname
                ));
            }
            // Measured coordinator quirk (trinodb/trino:483): `ALTER COLUMN
            // "c" DROP NOT NULL` with a quoted, already-lowercase column
            // name fails `Column '"c"' does not exist`, while the identical
            // statement with the name bare succeeds — every other ALTER
            // COLUMN form (`ADD COLUMN`, `DROP COLUMN`, `SET DATA TYPE`,
            // `RENAME COLUMN`) accepts the quoted form. `DROP NOT NULL`
            // alone is therefore emitted unquoted.
            stmts.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                qualified, name
            ));
            Ok(stmts)
        }
        SchemaOperation::AddStructField {
            column,
            path,
            field_name,
            field_type,
            default_expr,
        } => {
            let is_array_nested = path.iter().any(|p| p == "element");
            if !caps.supports_struct_field_ddl
                || (is_array_nested && !caps.supports_nested_array_ddl)
            {
                return Err(format!(
                    "Trino/Iceberg cannot add struct field '{}' to column '{}' — this \
                     catalog was not measured to accept a qualified ADD COLUMN path. \
                     Consider --allow-full-refresh",
                    field_name, column
                ));
            }
            if default_expr.is_some() {
                return Err(format!(
                    "Trino/Iceberg cannot backfill a default into newly added struct field \
                     '{}' on column '{}': there is no dotted UPDATE target \
                     (`UPDATE t SET {}.{} = …` is a parse error: `Expecting: '='`) and no \
                     column DEFAULT to fall back to",
                    field_name, column, column, field_name
                ));
            }
            let type_sql = trino_type_sql(field_type)
                .map_err(|why| format!("struct field '{}': {}", field_name, why))?;
            let dot_path = dotted_path(column, path, Some(field_name));
            Ok(vec![format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                qualified, dot_path, type_sql
            )])
        }
        SchemaOperation::RemoveStructField {
            column,
            path,
            field_name,
        } => {
            // Measured accepted — Iceberg's field-ID column tracking
            // (`supports_column_mapping`) makes a nested drop safe with no
            // rewrite, unlike Spark and BigQuery which both refuse this. A
            // catalog measured without that tracking has no safe form.
            if !caps.supports_column_mapping {
                return Err(format!(
                    "Trino/Iceberg cannot drop struct field '{}' from column '{}' on this \
                     catalog — column mapping (field-ID tracking) is required to make a \
                     nested drop safe. Consider --allow-full-refresh",
                    field_name, column
                ));
            }
            let dot_path = dotted_path(column, path, Some(field_name));
            Ok(vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                qualified, dot_path
            )])
        }
        SchemaOperation::WidenNestedType {
            column, path, to, ..
        } => {
            if path.len() == 1 && path[0] == "value" {
                // Map value widening — reconstruct the whole MAP type. The
                // key type is not carried by this operation, so (like
                // `ddl_duckdb`'s equivalent arm) an unbounded VARCHAR key
                // placeholder stands in; a non-VARCHAR-keyed map is planned
                // wrong here, an existing cross-backend gap this module
                // inherits rather than introduces (docs/specs/schema_evolution.md
                // §"Trino/Iceberg DDL").
                let new_map_type = DataType::Map(
                    Box::new(DataType::Varchar { max_length: None }),
                    Box::new(to.clone()),
                );
                let type_sql = trino_type_sql(&new_map_type)
                    .map_err(|why| format!("column '{}': {}", column, why))?;
                Ok(vec![format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET DATA TYPE {}",
                    qualified,
                    quote_ident(column),
                    type_sql
                )])
            } else {
                let is_array_nested = path.iter().any(|p| p == "element");
                if !path.is_empty()
                    && (!caps.supports_struct_field_ddl
                        || (is_array_nested && !caps.supports_nested_array_ddl))
                {
                    return Err(format!(
                        "Trino/Iceberg cannot widen the nested type at column '{}' (path: {}) \
                         on this catalog — this catalog was not measured to accept a \
                         qualified ALTER COLUMN path. Consider --allow-full-refresh",
                        column,
                        path.join(".")
                    ));
                }
                let type_sql =
                    trino_type_sql(to).map_err(|why| format!("column '{}': {}", column, why))?;
                let target = if path.is_empty() {
                    quote_ident(column)
                } else {
                    dotted_path(column, path, None)
                };
                Ok(vec![format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET DATA TYPE {}",
                    qualified, target, type_sql
                )])
            }
        }
        SchemaOperation::BackfillColumn { name, expression } => Ok(vec![format!(
            "UPDATE {} SET {} = {}",
            qualified,
            quote_ident(name),
            expression
        )]),
        SchemaOperation::RewriteColumn { column, .. } => Err(format!(
            "Trino/Iceberg has no ALTER COLUMN … USING, so column '{}' cannot be rewritten \
             in place — a two-step SET DATA TYPE then UPDATE is only sound when the target \
             type is already assignment-compatible, which WidenColumnType already covers",
            column
        )),
    }
}
