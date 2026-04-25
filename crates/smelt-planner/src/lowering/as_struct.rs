//! Phase 42 — `smelt.as_struct()` lowering helper.
//!
//! `as_struct_to_sql` emits backend-specific struct-literal SQL for a
//! `smelt.as_struct(alias [EXCEPT cols])` call. It is the lowering side of
//! the type-checker's `SmeltAsStructCall` handling; the type-checker still
//! lives in `smelt-db::function_body_check`, but the *SQL emission* is a
//! lowering concern and therefore belongs in the planner.
//!
//! Pure — no Salsa dependency. Callers pass the resolved field list and
//! target backend.

use smelt_types::DataType;

/// Returns `true` for backends that support struct literal syntax.
///
/// Supported: `duckdb`, `spark`, `databricks`, `postgres`.
/// Any other backend name returns `false`.
pub fn backend_supports_struct_literal(backend: &str) -> bool {
    matches!(
        backend.to_ascii_lowercase().as_str(),
        "duckdb" | "spark" | "databricks" | "postgres"
    )
}

/// Emit SQL for a `smelt.as_struct(alias [EXCEPT cols])` call against a
/// target backend.
///
/// Returns `Ok(sql)` for backends that support struct literals:
///   - DuckDB:    `{'col': alias.col, ...}`
///   - Spark / Databricks: `struct(alias.col AS col, ...)`
///   - Postgres:  `ROW(alias.col, ...)` (composite type; field names inferred)
///
/// Returns `Err(backend_name)` for backends without struct-literal support.
///
/// Pure — no Salsa dependency.
pub fn as_struct_to_sql(
    alias: &str,
    fields: &[(String, DataType)],
    backend: &str,
) -> Result<String, String> {
    if !backend_supports_struct_literal(backend) {
        return Err(backend.to_string());
    }
    let sql = match backend.to_ascii_lowercase().as_str() {
        "duckdb" => {
            let items: Vec<String> = fields
                .iter()
                .map(|(name, _)| format!("'{name}': {alias}.{name}"))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        "spark" | "databricks" => {
            let items: Vec<String> = fields
                .iter()
                .map(|(name, _)| format!("{alias}.{name} AS {name}"))
                .collect();
            format!("struct({})", items.join(", "))
        }
        "postgres" => {
            let items: Vec<String> = fields
                .iter()
                .map(|(name, _)| format!("{alias}.{name}"))
                .collect();
            format!("ROW({})", items.join(", "))
        }
        _ => unreachable!("backend_supports_struct_literal should have returned false"),
    };
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckdb_emits_brace_struct_literal() {
        let fields = vec![
            ("order_id".to_string(), DataType::BigInt),
            ("total".to_string(), DataType::Double),
        ];
        let sql = as_struct_to_sql("o", &fields, "duckdb").unwrap();
        assert_eq!(sql, "{'order_id': o.order_id, 'total': o.total}");
    }

    #[test]
    fn spark_emits_struct_constructor_with_aliases() {
        let fields = vec![
            ("order_id".to_string(), DataType::BigInt),
            ("total".to_string(), DataType::Double),
        ];
        let sql = as_struct_to_sql("o", &fields, "spark").unwrap();
        assert_eq!(sql, "struct(o.order_id AS order_id, o.total AS total)");
    }

    #[test]
    fn databricks_emits_same_as_spark() {
        let fields = vec![("id".to_string(), DataType::BigInt)];
        let sql = as_struct_to_sql("t", &fields, "databricks").unwrap();
        assert_eq!(sql, "struct(t.id AS id)");
    }

    #[test]
    fn postgres_emits_row_constructor() {
        let fields = vec![
            ("id".to_string(), DataType::BigInt),
            ("name".to_string(), DataType::Text),
        ];
        let sql = as_struct_to_sql("t", &fields, "postgres").unwrap();
        assert_eq!(sql, "ROW(t.id, t.name)");
    }

    #[test]
    fn unsupported_backend_returns_err() {
        let fields = vec![("id".to_string(), DataType::BigInt)];
        let err = as_struct_to_sql("t", &fields, "no_struct_db").unwrap_err();
        assert_eq!(err, "no_struct_db");
    }

    #[test]
    fn backend_capability_check_matches_emission() {
        // Every backend name that emits SQL must report capability=true.
        for backend in ["duckdb", "spark", "databricks", "postgres"] {
            assert!(
                backend_supports_struct_literal(backend),
                "{backend} should report capability"
            );
            let fields = vec![("x".to_string(), DataType::BigInt)];
            assert!(
                as_struct_to_sql("t", &fields, backend).is_ok(),
                "{backend} should emit SQL"
            );
        }

        for backend in ["no_struct_db", "sqlite", "redshift"] {
            assert!(
                !backend_supports_struct_literal(backend),
                "{backend} should not report capability"
            );
            let fields = vec![("x".to_string(), DataType::BigInt)];
            assert!(
                as_struct_to_sql("t", &fields, backend).is_err(),
                "{backend} should fail to emit"
            );
        }
    }
}
