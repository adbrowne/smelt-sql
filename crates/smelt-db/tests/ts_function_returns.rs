//! Timezone axis — Timezone-sensitive function return types and display surface.
//!
//! Verifies that:
//!   1. `NOW()` returns `Timestamp WITH TIME ZONE`, non-nullable.
//!   2. `CURRENT_TIMESTAMP` returns `Timestamp WITH TIME ZONE`, non-nullable.
//!   3. `MAKE_TIMESTAMPTZ(...)` returns `Timestamp WITH TIME ZONE`, nullable.
//!   4. `MAKE_TIMESTAMP(...)` (no TZ suffix) returns naive `Timestamp`, nullable (no regression).
//!   5. `DATE_TRUNC('day', ts_col)` over a naive Timestamp column returns naive `Timestamp`.
//!   6. `DATE_TRUNC('day', tstz_col)` over a `TIMESTAMPTZ` column returns `Timestamp WITH TIME ZONE`.
//!   7. `DataType::Timestamp { with_timezone: true }` displays as `"TIMESTAMP WITH TIME ZONE"` (hover regression guard).
//!   8. `DataType::Timestamp { with_timezone: false }` displays as `"TIMESTAMP"` (naive regression guard).
//!   9. `smelt.define` with `Expr<Timestamp WITH TIME ZONE>` parameter annotation produces no `InvalidFunctionTypeRef` diagnostic.

use std::path::PathBuf;

use std::sync::Arc;

use smelt_db::{
    file_diagnostics, typed_model_schema, Database, DiagnosticCode, ModelSchema, SourceFile,
    Workspace,
};
use smelt_types::{DataType, TypedColumn};

// ---------------------------------------------------------------------------
// Shared harness
// ---------------------------------------------------------------------------

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

/// Build a DB with a single model file and return its schema.
fn single_model_schema(sql: &str, test_name: &str) -> Arc<ModelSchema> {
    let root = PathBuf::from(format!("/fake/project/{}", test_name));
    let model_path = root.join("models").join("test_model.sql");
    let (db, ws, files) = build_db(root, &[(model_path, sql)]);
    typed_model_schema(&db, ws, files[0])
}

/// Extract the `TypedColumn` of the column named `col` from a `ModelSchema`.
fn col_type(schema: &ModelSchema, col: &str) -> TypedColumn {
    let c = schema
        .columns
        .iter()
        .find(|c| c.name == col)
        .unwrap_or_else(|| {
            panic!(
                "column '{}' not found in schema; columns: {:?}",
                col,
                schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
            )
        });
    c.data_type
        .as_ref()
        .unwrap_or_else(|| panic!("column '{}' has no inferred type", col))
        .clone()
}

// ---------------------------------------------------------------------------
// Test 1 — NOW() returns Timestamp WITH TIME ZONE, non-nullable
// ---------------------------------------------------------------------------

#[test]
fn now_returns_timestamptz() {
    let schema = single_model_schema("SELECT NOW() AS ts\n", "now_tz");
    let typed = col_type(&schema, "ts");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: true
        },
        "NOW() must return Timestamp WITH TIME ZONE, got {:?}",
        typed.data_type
    );
    assert!(
        !typed.nullable,
        "NOW() must be non-nullable, got nullable={}",
        typed.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 2 — CURRENT_TIMESTAMP returns Timestamp WITH TIME ZONE, non-nullable
// ---------------------------------------------------------------------------

#[test]
fn current_timestamp_returns_timestamptz() {
    let schema = single_model_schema("SELECT CURRENT_TIMESTAMP AS ts\n", "current_ts_tz");
    let typed = col_type(&schema, "ts");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: true
        },
        "CURRENT_TIMESTAMP must return Timestamp WITH TIME ZONE, got {:?}",
        typed.data_type
    );
    assert!(
        !typed.nullable,
        "CURRENT_TIMESTAMP must be non-nullable, got nullable={}",
        typed.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 3 — MAKE_TIMESTAMPTZ returns Timestamp WITH TIME ZONE, nullable
// ---------------------------------------------------------------------------

#[test]
fn make_timestamptz_returns_timestamptz() {
    let schema = single_model_schema(
        "SELECT MAKE_TIMESTAMPTZ(2024, 1, 1, 0, 0, 0) AS ts\n",
        "make_timestamptz",
    );
    let typed = col_type(&schema, "ts");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: true
        },
        "MAKE_TIMESTAMPTZ must return Timestamp WITH TIME ZONE, got {:?}",
        typed.data_type
    );
    assert!(
        typed.nullable,
        "MAKE_TIMESTAMPTZ must be nullable, got nullable={}",
        typed.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 4 — MAKE_TIMESTAMP (no TZ suffix) returns naive Timestamp, nullable
// ---------------------------------------------------------------------------

#[test]
fn make_timestamp_returns_naive() {
    let schema = single_model_schema(
        "SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts\n",
        "make_timestamp_naive",
    );
    let typed = col_type(&schema, "ts");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: false
        },
        "MAKE_TIMESTAMP must return naive Timestamp (with_timezone=false), got {:?}",
        typed.data_type
    );
    assert!(
        typed.nullable,
        "MAKE_TIMESTAMP must be nullable, got nullable={}",
        typed.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 5 — DATE_TRUNC over a naive Timestamp column preserves naive
// ---------------------------------------------------------------------------

#[test]
fn date_trunc_preserves_naive() {
    // Upstream model with a naive TIMESTAMP column.
    let root = PathBuf::from("/fake/project/date_trunc_naive");
    let upstream_path = root.join("models").join("upstream.sql");
    // MAKE_TIMESTAMP returns naive Timestamp — use it to get a typed ts_col.
    let upstream_src = "SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts_col\n";

    let model_path = root.join("models").join("trunc_model.sql");
    let model_src = "SELECT DATE_TRUNC('day', ts_col) AS ts_trunc FROM smelt.models.upstream\n";

    let (db, ws, files) = build_db(
        root,
        &[(upstream_path, upstream_src), (model_path, model_src)],
    );

    let schema = typed_model_schema(&db, ws, files[1]);
    let typed = col_type(&schema, "ts_trunc");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: false
        },
        "DATE_TRUNC over naive Timestamp must return naive Timestamp, got {:?}",
        typed.data_type
    );
}

// ---------------------------------------------------------------------------
// Test 6 — DATE_TRUNC over a TIMESTAMPTZ column preserves tz-aware
// ---------------------------------------------------------------------------

#[test]
fn date_trunc_preserves_timestamptz() {
    // Upstream model with a TIMESTAMPTZ column.
    let root = PathBuf::from("/fake/project/date_trunc_timestamptz");
    let upstream_path = root.join("models").join("upstream.sql");
    // NOW() returns Timestamp WITH TIME ZONE — use it as our tstz_col source.
    let upstream_src = "SELECT NOW() AS tstz_col\n";

    let model_path = root.join("models").join("trunc_model.sql");
    let model_src = "SELECT DATE_TRUNC('day', tstz_col) AS ts_trunc FROM smelt.models.upstream\n";

    let (db, ws, files) = build_db(
        root,
        &[(upstream_path, upstream_src), (model_path, model_src)],
    );

    let schema = typed_model_schema(&db, ws, files[1]);
    let typed = col_type(&schema, "ts_trunc");
    assert_eq!(
        typed.data_type,
        DataType::Timestamp {
            with_timezone: true
        },
        "DATE_TRUNC over Timestamp WITH TIME ZONE must return Timestamp WITH TIME ZONE, got {:?}",
        typed.data_type
    );
}

// ---------------------------------------------------------------------------
// Test 7 — Timestamp WITH TIME ZONE hover string (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn timestamptz_hover_string() {
    // Verifies the canonical display/hover string for a tz-aware Timestamp.
    // This is a regression guard: the string "TIMESTAMP WITH TIME ZONE" must
    // appear verbatim so that hover text, diagnostic messages, and
    // smelt.define annotation annotations all render consistently.
    let rendered = DataType::Timestamp {
        with_timezone: true,
    }
    .to_string();
    assert_eq!(
        rendered, "TIMESTAMP WITH TIME ZONE",
        "Timestamp WITH TIME ZONE must render as 'TIMESTAMP WITH TIME ZONE', got '{}'",
        rendered
    );
}

// ---------------------------------------------------------------------------
// Test 8 — Naive Timestamp hover string (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn timestamp_hover_string() {
    // Verifies the canonical display/hover string for a naive (tz-unaware) Timestamp.
    // Must render without any timezone suffix to match DuckDB's TIMESTAMP type.
    let rendered = DataType::Timestamp {
        with_timezone: false,
    }
    .to_string();
    assert_eq!(
        rendered, "TIMESTAMP",
        "Naive Timestamp must render as 'TIMESTAMP', got '{}'",
        rendered
    );
}

// ---------------------------------------------------------------------------
// Test 9 — smelt.define with Expr<Timestamp WITH TIME ZONE> produces no
//           InvalidFunctionTypeRef (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn define_signature_timestamptz_parses() {
    // A smelt.define function with an `Expr<Timestamp WITH TIME ZONE>` parameter
    // annotation must not produce an `InvalidFunctionTypeRef` (or any error)
    // diagnostic. The parse path in smelt-types/src/parse.rs recognises both
    // `TIMESTAMPTZ` and `TIMESTAMP WITH TIME ZONE` as valid type annotations.
    let root = PathBuf::from("/fake/project/define_sig_tstz");
    let fn_path = root.join("functions").join("format_ts.sql");
    let fn_src =
        "smelt.define format_ts(ts: Expr<Timestamp WITH TIME ZONE>) -> Expr<Text> AS (CAST(ts AS VARCHAR))\n";

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let sf = db.set_source_file(fn_path, fn_src.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();

    let diags = file_diagnostics(&db, ws, sf);
    let type_ref_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert!(
        type_ref_errors.is_empty(),
        "smelt.define with Expr<Timestamp WITH TIME ZONE> must not produce InvalidFunctionTypeRef; \
         got: {type_ref_errors:#?}"
    );
}
