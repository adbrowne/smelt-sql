//! Live-gated proof that `ddl_trino`'s statements are what the mapping table
//! in `crates/smelt-state/src/ddl_trino/mod.rs` claims: a row that claims a
//! statement executes against a fresh live Iceberg table and leaves the
//! schema the operation intended; a row that claims `FullRefreshRequired`
//! has the corresponding raw statement refused by the coordinator.
//!
//! Skips green when `SMELT_TRINO_URL` is unset. Run it with:
//!   bash scripts/trino-up.sh && source scripts/trino-env.sh
//!   cargo test -p smelt-cli --test trino_ddl_live

mod common;
use common::{trino_backend, trino_env, trino_schema};

use smelt_backend::Backend;
use smelt_dialect::BackendCapabilities;
use smelt_state::ddl_trino::{generate_trino_ddl, TrinoMigration};
use smelt_state::schema_tracking::SchemaOperation;
use smelt_types::DataType;

fn exec(backend: &smelt_backend_trino::TrinoBackend, sql: &str) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async { backend.execute_sql(sql).await })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `DESCRIBE schema.table`'s `(Column, Type)` pairs, for asserting a
/// migration actually left the schema the operation intended.
fn describe(backend: &smelt_backend_trino::TrinoBackend, table: &str) -> Vec<(String, String)> {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let batches = rt
        .block_on(async { backend.execute_sql(&format!("DESCRIBE {table}")).await })
        .unwrap_or_else(|e| panic!("DESCRIBE {table} failed: {e}"));
    let mut out = Vec::new();
    for batch in &batches {
        let names = arrow::array::as_string_array(batch.column(0));
        let types = arrow::array::as_string_array(batch.column(1));
        for i in 0..batch.num_rows() {
            out.push((names.value(i).to_string(), types.value(i).to_string()));
        }
    }
    out
}

fn caps() -> BackendCapabilities {
    BackendCapabilities::trino_iceberg()
}

fn run_stmts(backend: &smelt_backend_trino::TrinoBackend, stmts: &[String]) {
    for stmt in stmts {
        exec(backend, stmt).unwrap_or_else(|e| panic!("statement `{stmt}` failed: {e}"));
    }
}

#[test]
fn ddl_trino_mapping_table_matches_the_live_coordinator() {
    let Some(env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping ddl_trino_mapping_table_matches_the_live_coordinator"
        );
        return;
    };
    let schema = trino_schema("ddl_live");
    let backend = trino_backend(&schema);
    let catalog = env.catalog.clone();
    let qschema = format!("\"{catalog}\".\"{schema}\"");
    exec(&backend, &format!("CREATE SCHEMA IF NOT EXISTS {qschema}")).unwrap();

    let cases_run = std::cell::Cell::new(0usize);

    macro_rules! fresh_table {
        ($leaf:expr, $defs:expr) => {{
            let t = format!("{qschema}.\"{}\"", $leaf);
            exec(&backend, &format!("CREATE TABLE {t} ({})", $defs)).unwrap();
            t
        }};
    }

    // ADD COLUMN, nullable, no default — Statements, executes, DESCRIBE shows it.
    {
        let table = fresh_table!("add_col", "id INTEGER");
        let ops = vec![SchemaOperation::AddColumn {
            name: "amount".into(),
            data_type: DataType::BigInt,
            nullable: true,
            default_expr: None,
        }];
        match generate_trino_ddl(&catalog, &schema, "add_col", &ops, &caps()) {
            TrinoMigration::Statements(stmts) => {
                run_stmts(&backend, &stmts);
                let cols = describe(&backend, &table);
                assert!(
                    cols.iter().any(|(n, t)| n == "amount" && t == "bigint"),
                    "expected amount:bigint in {cols:?}"
                );
            }
            other => panic!("expected statements, got {:?}", other),
        }
        cases_run.set(cases_run.get() + 1);
    }

    // ADD COLUMN NOT NULL — always FullRefreshRequired; the raw refused form
    // asserted separately below.
    {
        let ops = vec![SchemaOperation::AddColumn {
            name: "amount".into(),
            data_type: DataType::BigInt,
            nullable: false,
            default_expr: None,
        }];
        match generate_trino_ddl(&catalog, &schema, "irrelevant", &ops, &caps()) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("amount"), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
        let t = fresh_table!("add_notnull", "id INTEGER");
        let err = exec(
            &backend,
            &format!("ALTER TABLE {t} ADD COLUMN amount BIGINT NOT NULL"),
        )
        .expect_err("the coordinator must refuse ADD COLUMN ... NOT NULL");
        assert!(
            err.contains("not null") || err.to_lowercase().contains("not null"),
            "{err}"
        );
        cases_run.set(cases_run.get() + 1);
    }

    // DROP COLUMN, top-level.
    {
        let table = fresh_table!("drop_col", "id INTEGER, amount BIGINT");
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "amount".into(),
        }];
        match generate_trino_ddl(&catalog, &schema, "drop_col", &ops, &caps()) {
            TrinoMigration::Statements(stmts) => {
                run_stmts(&backend, &stmts);
                let cols = describe(&backend, &table);
                assert!(
                    !cols.iter().any(|(n, _)| n == "amount"),
                    "amount must be gone: {cols:?}"
                );
            }
            other => panic!("expected statements, got {:?}", other),
        }
        cases_run.set(cases_run.get() + 1);
    }

    // WidenColumnType.
    {
        let table = fresh_table!("widen", "id INTEGER, amount INTEGER");
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        match generate_trino_ddl(&catalog, &schema, "widen", &ops, &caps()) {
            TrinoMigration::Statements(stmts) => {
                run_stmts(&backend, &stmts);
                let cols = describe(&backend, &table);
                assert!(
                    cols.iter().any(|(n, t)| n == "amount" && t == "bigint"),
                    "{cols:?}"
                );
            }
            other => panic!("expected statements, got {:?}", other),
        }
        cases_run.set(cases_run.get() + 1);
    }

    // ChangeNullability, both directions.
    {
        let table = fresh_table!("nullability", "id INTEGER, amount BIGINT NOT NULL");
        let relax = vec![SchemaOperation::ChangeNullability {
            name: "amount".into(),
            to_nullable: true,
            default_expr: None,
        }];
        match generate_trino_ddl(&catalog, &schema, "nullability", &relax, &caps()) {
            TrinoMigration::Statements(stmts) => run_stmts(&backend, &stmts),
            other => panic!("expected statements, got {:?}", other),
        }

        let tighten = vec![SchemaOperation::ChangeNullability {
            name: "amount".into(),
            to_nullable: false,
            default_expr: None,
        }];
        match generate_trino_ddl(&catalog, &schema, "nullability", &tighten, &caps()) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("SET NOT NULL"), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
        let err = exec(
            &backend,
            &format!("ALTER TABLE {table} ALTER COLUMN amount SET NOT NULL"),
        )
        .expect_err("the coordinator must refuse SET NOT NULL");
        assert!(!err.is_empty());
        cases_run.set(cases_run.get() + 1);
    }

    // AddStructField (no default) and RemoveStructField.
    {
        let table = fresh_table!("struct_add", "id INTEGER, meta ROW(a INTEGER)");
        let add = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Text,
            default_expr: None,
        }];
        match generate_trino_ddl(&catalog, &schema, "struct_add", &add, &caps()) {
            TrinoMigration::Statements(stmts) => run_stmts(&backend, &stmts),
            other => panic!("expected statements, got {:?}", other),
        }
        let cols = describe(&backend, &table);
        assert!(
            cols.iter().any(|(n, _)| n == "meta"),
            "meta column must still exist: {cols:?}"
        );

        let remove = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
        }];
        match generate_trino_ddl(&catalog, &schema, "struct_add", &remove, &caps()) {
            TrinoMigration::Statements(stmts) => run_stmts(&backend, &stmts),
            other => panic!("expected statements, got {:?}", other),
        }
        cases_run.set(cases_run.get() + 1);
    }

    // AddStructField with a default — always refused; raw dotted UPDATE
    // confirmed refused too.
    {
        let table = fresh_table!("struct_default", "id INTEGER, meta ROW(a INTEGER)");
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Integer,
            default_expr: Some("0".into()),
        }];
        match generate_trino_ddl(&catalog, &schema, "struct_default", &ops, &caps()) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("meta") && reason.contains('b'), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
        let err = exec(&backend, &format!("UPDATE {table} SET meta.b = 0"))
            .expect_err("dotted UPDATE assignment must be refused");
        assert!(!err.is_empty());
        cases_run.set(cases_run.get() + 1);
    }

    // WidenNestedType — struct field.
    {
        let table = fresh_table!("widen_nested", "id INTEGER, meta ROW(a INTEGER)");
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        match generate_trino_ddl(&catalog, &schema, "widen_nested", &ops, &caps()) {
            TrinoMigration::Statements(stmts) => run_stmts(&backend, &stmts),
            other => panic!("expected statements, got {:?}", other),
        }
        let cols = describe(&backend, &table);
        assert!(
            cols.iter()
                .any(|(n, t)| n == "meta" && t.contains("bigint")),
            "{cols:?}"
        );
        cases_run.set(cases_run.get() + 1);
    }

    // BackfillColumn.
    {
        let table = fresh_table!("backfill", "id INTEGER, amount BIGINT");
        exec(
            &backend,
            &format!("INSERT INTO {table} VALUES (1, NULL), (2, NULL)"),
        )
        .unwrap();
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "amount".into(),
            expression: "0".into(),
        }];
        match generate_trino_ddl(&catalog, &schema, "backfill", &ops, &caps()) {
            TrinoMigration::Statements(stmts) => run_stmts(&backend, &stmts),
            other => panic!("expected statements, got {:?}", other),
        }
        cases_run.set(cases_run.get() + 1);
    }

    // RewriteColumn — always refused; raw USING form confirmed refused too.
    {
        let table = fresh_table!("rewrite", "id INTEGER, amount VARCHAR");
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "amount".into(),
            target_type: DataType::Integer,
            using_expr: "CAST(amount AS INTEGER)".into(),
        }];
        match generate_trino_ddl(&catalog, &schema, "rewrite", &ops, &caps()) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("amount") && reason.contains("USING"),
                    "{reason}"
                );
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
        let err = exec(
            &backend,
            &format!("ALTER TABLE {table} ALTER COLUMN amount SET DATA TYPE INTEGER USING CAST(amount AS INTEGER)"),
        )
        .expect_err("the coordinator must refuse a USING clause");
        assert!(!err.is_empty());
        cases_run.set(cases_run.get() + 1);
    }

    // Type spellings — one table exercising every candidate spelling.
    {
        let type_cases: &[(&str, DataType)] = &[
            ("t_bool", DataType::Boolean),
            ("t_smallint", DataType::SmallInt),
            ("t_int", DataType::Integer),
            ("t_bigint", DataType::BigInt),
            ("t_float", DataType::Float),
            ("t_double", DataType::Double),
            (
                "t_decimal",
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
            ("t_varchar", DataType::Varchar { max_length: None }),
            (
                "t_varchar_n",
                DataType::Varchar {
                    max_length: Some(20),
                },
            ),
            ("t_char", DataType::Char { length: 3 }),
            ("t_text", DataType::Text),
            ("t_blob", DataType::Blob),
            ("t_date", DataType::Date),
            ("t_time", DataType::Time),
            (
                "t_timestamp",
                DataType::Timestamp {
                    with_timezone: false,
                },
            ),
            (
                "t_timestamptz",
                DataType::Timestamp {
                    with_timezone: true,
                },
            ),
            ("t_array", DataType::Array(Box::new(DataType::Integer))),
            (
                "t_row",
                DataType::Struct(vec![("a".to_string(), DataType::Integer)]),
            ),
            (
                "t_map",
                DataType::Map(Box::new(DataType::Text), Box::new(DataType::Integer)),
            ),
        ];
        let defs: Vec<String> = type_cases
            .iter()
            .map(|(name, dt)| {
                format!(
                    "{name} {}",
                    smelt_state::ddl_trino::trino_type_sql(dt).unwrap()
                )
            })
            .collect();
        let table = fresh_table!("type_spellings", defs.join(", "));
        let cols = describe(&backend, &table);
        assert_eq!(cols.len(), type_cases.len(), "{cols:?}");
        cases_run.set(cases_run.get() + 1);
    }

    assert!(
        cases_run.get() >= 10,
        "expected at least 10 live cases to have executed, got {}",
        cases_run.get()
    );
    eprintln!(
        "ddl_trino_mapping_table_matches_the_live_coordinator: {} cases executed",
        cases_run.get()
    );

    exec(
        &backend,
        &format!("DROP SCHEMA IF EXISTS {qschema} CASCADE"),
    )
    .ok();
}
