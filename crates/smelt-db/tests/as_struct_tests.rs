//! Phase 38 (smelt-functions) — `smelt.as_struct()` revisit.
//!
//! Tests verify:
//!   1. `as_struct_to_sql` emits a DuckDB struct literal for basic fields.
//!   2. `smelt.as_struct(alias EXCEPT col)` filters out the specified
//!      column from the resolved struct type in a model.
//!   3. `as_struct_to_sql` emits the correct SQL for DuckDB, Spark, and
//!      Postgres backends.
//!   4. Two `smelt.as_struct()` calls with different aliases in a
//!      multi-join model each resolve the correct columns without
//!      cross-contamination.
//!   5. A function whose declared backend set includes a backend without
//!      struct-literal support emits `AsStructUnsupportedBackend`.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn build_db(
    project_root: PathBuf,
    sources_yaml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), sources_yaml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

// ─── Test 1: as_struct_to_sql emits DuckDB struct literal ─────────────────────

#[test]
fn as_struct_basic_emits_struct_literal() {
    // TDD test 1: `as_struct_to_sql("t", [(id, BigInt), (name, Text)], "duckdb")`
    // should return a DuckDB-style struct literal `{'id': t.id, 'name': t.name}`.
    use smelt_db::function_body_check::as_struct_to_sql;
    use smelt_types::DataType;

    let fields = vec![
        ("id".to_string(), DataType::BigInt),
        ("name".to_string(), DataType::Text),
    ];
    let sql =
        as_struct_to_sql("t", &fields, "duckdb").expect("duckdb must support struct literals");

    assert!(
        sql.contains("'id': t.id"),
        "expected DuckDB field `'id': t.id`; got `{sql}`"
    );
    assert!(
        sql.contains("'name': t.name"),
        "expected DuckDB field `'name': t.name`; got `{sql}`"
    );
}

// ─── Test 2: as_struct EXCEPT filters columns ──────────────────────────────────

#[test]
fn as_struct_except_filters_columns() {
    // TDD test 2: in a model that does `smelt.as_struct(e EXCEPT ts)` on a
    // source with {ts: Timestamp, user_id: Integer, amount: Numeric}, the
    // resulting column `s` should be a Struct containing user_id and amount
    // but NOT ts.
    use smelt_db::typed_model_schema;
    use smelt_types::DataType;

    let root = PathBuf::from("/fake/project38b");
    let model_path = root.join("models").join("test_except.sql");
    let model_src = "SELECT smelt.as_struct(e EXCEPT ts) AS s \
                     FROM smelt.sources.source.events AS e\n";
    let sources_yaml = "version: 1\nsources:\n  source:\n    tables:\n      events:\n        columns:\n          - name: ts\n            type: TIMESTAMP\n          - name: user_id\n            type: INTEGER\n          - name: amount\n            type: NUMERIC\n";

    let (db, ws, files) = build_db(root, sources_yaml, &[(model_path, model_src)]);
    let model_file = files[0];

    // Zero type-errors expected.
    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownIdentifier) | Some(DiagnosticCode::TypeMismatch)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "smelt.as_struct with EXCEPT should be clean; got {bad:#?}"
    );

    // The resulting column `s` should have type Struct with user_id and
    // amount, but NOT ts.
    let schema = typed_model_schema(&db, ws, model_file);
    let s_col = schema.columns.iter().find(|c| c.name == "s");
    assert!(
        s_col.is_some(),
        "expected column `s` in schema; got {schema:#?}"
    );
    let s_type = s_col
        .unwrap()
        .data_type
        .as_ref()
        .map(|tc| tc.data_type.clone())
        .unwrap_or(DataType::Unknown);

    match &s_type {
        DataType::Struct(fields) => {
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(
                !names.contains(&"ts"),
                "EXCEPT ts should exclude `ts`; got {names:?}"
            );
            assert!(
                names.contains(&"user_id"),
                "expected `user_id` in struct; got {names:?}"
            );
            assert!(
                names.contains(&"amount"),
                "expected `amount` in struct; got {names:?}"
            );
        }
        other => panic!("expected DataType::Struct for `s`; got {other:?}"),
    }
}

// ─── Test 3: backend printer emits DuckDB / Spark / Postgres ──────────────────

#[test]
fn as_struct_backend_printer_emits_duckdb_spark_postgres() {
    // TDD test 3: `as_struct_to_sql` must emit the correct struct literal
    // syntax for three backends:
    //   DuckDB:   `{'id': t.id, 'name': t.name}`
    //   Spark:    `struct(t.id AS id, t.name AS name)`
    //   Postgres: `ROW(t.id, t.name)` (loses field names but is valid SQL)
    use smelt_db::function_body_check::as_struct_to_sql;
    use smelt_types::DataType;

    let fields = vec![
        ("id".to_string(), DataType::BigInt),
        ("name".to_string(), DataType::Text),
    ];

    // DuckDB
    let duckdb_sql = as_struct_to_sql("t", &fields, "duckdb").expect("duckdb must succeed");
    assert!(
        duckdb_sql.contains("'id': t.id"),
        "DuckDB: expected `'id': t.id`; got `{duckdb_sql}`"
    );
    assert!(
        duckdb_sql.contains("'name': t.name"),
        "DuckDB: expected `'name': t.name`; got `{duckdb_sql}`"
    );

    // Spark
    let spark_sql = as_struct_to_sql("t", &fields, "spark").expect("spark must succeed");
    assert!(
        spark_sql.starts_with("struct("),
        "Spark: expected `struct(...)` form; got `{spark_sql}`"
    );
    assert!(
        spark_sql.contains("t.id AS id"),
        "Spark: expected `t.id AS id`; got `{spark_sql}`"
    );
    assert!(
        spark_sql.contains("t.name AS name"),
        "Spark: expected `t.name AS name`; got `{spark_sql}`"
    );

    // Postgres
    let pg_sql = as_struct_to_sql("t", &fields, "postgres").expect("postgres must succeed");
    assert!(
        pg_sql.starts_with("ROW("),
        "Postgres: expected `ROW(...)` form; got `{pg_sql}`"
    );
    assert!(
        pg_sql.contains("t.id"),
        "Postgres: expected `t.id` in ROW; got `{pg_sql}`"
    );
    assert!(
        pg_sql.contains("t.name"),
        "Postgres: expected `t.name` in ROW; got `{pg_sql}`"
    );
}

// ─── Test 4: multi-join context resolves without collision ────────────────────

#[test]
fn as_struct_in_multi_join_context_resolves_without_collision() {
    // TDD test 4: §6 Strategy 3 use case. A model joins two sources and
    // uses `smelt.as_struct()` on each to prevent column-name collisions.
    //   smelt.as_struct(o EXCEPT customer_id) → Struct(order_id, total)
    //   smelt.as_struct(c EXCEPT customer_id) → Struct(name, tier)
    // Both must resolve zero diagnostics, and the order_data struct must
    // NOT include customer_id.
    use smelt_db::typed_model_schema;
    use smelt_types::DataType;

    let root = PathBuf::from("/fake/project38d");
    let model_path = root.join("models").join("multi_join.sql");
    let model_src = "\
SELECT \
  smelt.as_struct(o EXCEPT customer_id) AS order_data, \
  smelt.as_struct(c EXCEPT customer_id) AS customer_data \
FROM smelt.sources.source.orders AS o \
JOIN smelt.sources.source.customers AS c \
  ON o.customer_id = c.customer_id\n";
    let sources_yaml = "version: 1\nsources:\n  source:\n    tables:\n      orders:\n        columns:\n          - name: order_id\n            type: BIGINT\n          - name: customer_id\n            type: VARCHAR\n          - name: total\n            type: NUMERIC\n      customers:\n        columns:\n          - name: customer_id\n            type: VARCHAR\n          - name: name\n            type: VARCHAR\n          - name: tier\n            type: VARCHAR\n";

    let (db, ws, files) = build_db(root, sources_yaml, &[(model_path, model_src)]);
    let model_file = files[0];

    // Zero type errors expected.
    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownIdentifier) | Some(DiagnosticCode::TypeMismatch)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "multi-join as_struct should produce no errors; got {bad:#?}"
    );

    // order_data should be Struct(order_id, total) — no customer_id.
    let schema = typed_model_schema(&db, ws, model_file);
    let order_col = schema.columns.iter().find(|c| c.name == "order_data");
    assert!(
        order_col.is_some(),
        "expected `order_data` in schema; got {schema:#?}"
    );
    let order_type = order_col
        .unwrap()
        .data_type
        .as_ref()
        .map(|tc| tc.data_type.clone())
        .unwrap_or(DataType::Unknown);
    match &order_type {
        DataType::Struct(fields) => {
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(
                !names.contains(&"customer_id"),
                "EXCEPT customer_id must not appear; got {names:?}"
            );
            assert!(
                names.contains(&"order_id"),
                "expected `order_id` in struct; got {names:?}"
            );
            assert!(
                names.contains(&"total"),
                "expected `total` in struct; got {names:?}"
            );
        }
        other => panic!("expected DataType::Struct for order_data; got {other:?}"),
    }
}

// ─── Test 5: backend without struct literal emits diagnostic ──────────────────

#[test]
fn backend_without_struct_literal_errors() {
    // TDD test 5: a function whose `backends: [no_struct_db]` uses
    // `smelt.as_struct()` in its body must produce
    // `AsStructUnsupportedBackend` naming the offending backend.
    let root = PathBuf::from("/fake/project38e");
    let fn_path = root.join("models").join("fn_no_struct.sql");
    let fn_src = "\
---\n\
backends: [no_struct_db]\n\
---\n\
smelt.define fn_no_struct(source: TableExpr) -> TableExpr AS (\n\
    SELECT smelt.as_struct(source) AS s FROM source\n\
)\n";

    let (db, ws, files) = build_db(root, "", &[(fn_path, fn_src)]);
    let fn_file = files[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let as_struct_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::AsStructUnsupportedBackend)
                && d.message.contains("no_struct_db")
        })
        .collect();
    assert!(
        !as_struct_diags.is_empty(),
        "expected AsStructUnsupportedBackend mentioning `no_struct_db`; got {diags:#?}"
    );
}
