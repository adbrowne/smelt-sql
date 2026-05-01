//! Phase 37 (smelt-functions) — Row variables in return types with concrete erasure.
//!
//! Tests verify:
//!   1. `with_hour` function (Struct return with row var) produces zero diagnostics
//!      when called with a struct argument that has the declared fields plus extras.
//!   2. The concrete return type at a call site with `{ts: Timestamp, user_id: Integer}`
//!      resolves to `DataType::Struct([("hour", BigInt), ("user_id", Integer)])`.
//!   3. The resolved return type string shows concrete fields — not the row variable `..r`.
//!   4. Spread items (`..event`) in the body expand to explicit field references for the
//!      extras bound to the row var.

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

/// Canonical `with_hour` definition with a struct return type containing `..r`.
///
/// Takes `event: Expr<Struct<{ts: Timestamp, ..r}>>` and returns
/// `Expr<Struct<{hour: BigInt, ..r}>>`.  At a call site with extras
/// `[(user_id, Integer)]`, the return resolves to
/// `Struct<{hour: BigInt, user_id: Integer}>`.
const WITH_HOUR_SRC: &str =
    "smelt.define with_hour(\n    event: Expr<Struct<{ts: Timestamp, ..r}>>\n) -> Expr<Struct<{hour: BigInt, ..r}>> AS (\n    {EXTRACT(HOUR FROM event.ts) AS hour, ..event}\n)\n";

/// A minimal YAML that declares a `source.events` table with `ts: TIMESTAMP`
/// and `user_id: INTEGER`.
const SOURCES_WITH_TS_USER: &str = "version: 1\nsources:\n  source:\n    tables:\n      events:\n        columns:\n          - name: ts\n            type: TIMESTAMP\n          - name: user_id\n            type: INTEGER\n";

// ─── Test 1: with_hour produces zero diagnostics ──────────────────────────────

#[test]
fn with_hour_types_clean() {
    // TDD test 1: call `with_hour` on a source that has `ts: Timestamp`
    // plus `user_id: Integer`. The declared field `ts` is present and
    // type-compatible; `user_id` is extra and captured by the `..r` tail.
    // The return type `Expr<Struct<{hour: BigInt, ..r}>>` should also resolve.
    // Zero error diagnostics expected.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_hour.sql");
    let model_path = root.join("models").join("event_with_hour.sql");
    let model_src = "SELECT smelt.fn.with_hour(e) AS ev \
                     FROM smelt.sources.source.events AS e\n";

    let (db, ws, files) = build_db(
        root,
        SOURCES_WITH_TS_USER,
        &[(fn_path, WITH_HOUR_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
                    | Some(DiagnosticCode::RowRequirementUnsatisfied)
                    | Some(DiagnosticCode::InvalidFunctionTypeRef)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "with_hour with ts+user_id should be clean; got {bad:#?}"
    );
}

// ─── Test 2: return row var binds to param row var ─────────────────────────

#[test]
fn return_row_var_binds_to_param_row_var() {
    // TDD test 2: call `with_hour` on `{ts: Timestamp, user_id: Integer}`.
    // The return type `Expr<Struct<{hour: BigInt, ..r}>>` should resolve to
    // `DataType::Struct([("hour", BigInt), ("user_id", Integer)])`.
    //
    // Tested by inspecting the typed model schema: the column `ev` should
    // have type `STRUCT(hour BIGINT, user_id INTEGER)`.
    use smelt_db::typed_model_schema;
    use smelt_types::DataType;

    let root = PathBuf::from("/fake/project37b");
    let fn_path = root.join("functions").join("with_hour.sql");
    let model_path = root.join("models").join("event_with_hour.sql");
    let model_src = "SELECT smelt.fn.with_hour(e) AS ev \
                     FROM smelt.sources.source.events AS e\n";

    let (db, ws, files) = build_db(
        root,
        SOURCES_WITH_TS_USER,
        &[(fn_path, WITH_HOUR_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let schema = typed_model_schema(&db, ws, model_file);
    let ev_col = schema.columns.iter().find(|c| c.name == "ev");
    assert!(
        ev_col.is_some(),
        "expected column `ev` in schema; got {schema:#?}"
    );

    let ev_typed_col = ev_col.unwrap().data_type.clone();
    assert!(
        ev_typed_col.is_some(),
        "expected `ev` to have an inferred type; got None"
    );
    let ev_type = ev_typed_col.unwrap().data_type;

    // The resolved return type must be a Struct containing both `hour` and `user_id`.
    match &ev_type {
        DataType::Struct(fields) => {
            let field_names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(
                field_names.contains(&"hour"),
                "expected `hour` field in struct; got {field_names:?}"
            );
            assert!(
                field_names.contains(&"user_id"),
                "expected `user_id` field in struct; got {field_names:?}"
            );
            // Check types too
            let hour_type = fields.iter().find(|(n, _)| n == "hour").map(|(_, dt)| dt);
            let user_id_type = fields
                .iter()
                .find(|(n, _)| n == "user_id")
                .map(|(_, dt)| dt);
            assert_eq!(
                hour_type,
                Some(&DataType::BigInt),
                "expected `hour` to be BigInt; got {hour_type:?}"
            );
            assert_eq!(
                user_id_type,
                Some(&DataType::Integer),
                "expected `user_id` to be Integer; got {user_id_type:?}"
            );
        }
        other => panic!("expected DataType::Struct for `ev`; got {other:?}"),
    }
}

// ─── Test 3: caller sees fully resolved return type ───────────────────────────

#[test]
fn caller_sees_fully_resolved_return_type() {
    // TDD test 3: the resolved return type of `with_hour` at a call site
    // with `{ts: Timestamp, user_id: Integer}` must NOT contain `..r` —
    // it must show concrete fields `hour` and `user_id`.
    //
    // Tested by inspecting the DataType::Struct display string from the
    // typed model schema, verifying it contains `hour` and `user_id` but
    // not the row var name `r`.
    use smelt_db::typed_model_schema;
    use smelt_types::DataType;

    let root = PathBuf::from("/fake/project37c");
    let fn_path = root.join("functions").join("with_hour.sql");
    let model_path = root.join("models").join("event_with_hour.sql");
    let model_src = "SELECT smelt.fn.with_hour(e) AS ev \
                     FROM smelt.sources.source.events AS e\n";

    let (db, ws, files) = build_db(
        root,
        SOURCES_WITH_TS_USER,
        &[(fn_path, WITH_HOUR_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let schema = typed_model_schema(&db, ws, model_file);
    let ev_col = schema.columns.iter().find(|c| c.name == "ev");
    assert!(ev_col.is_some(), "expected column `ev` in schema");

    let ev_type = ev_col
        .unwrap()
        .data_type
        .clone()
        .map(|tc| tc.data_type)
        .unwrap_or(DataType::Unknown);
    let type_str = ev_type.to_string();

    // The type string must show concrete fields, not the row variable.
    assert!(
        type_str.contains("hour"),
        "resolved type should contain `hour` field; got `{type_str}`"
    );
    assert!(
        type_str.contains("user_id"),
        "resolved type should contain `user_id` field; got `{type_str}`"
    );
    // Must NOT contain the raw row-var name `r` as a type construct.
    // The struct display is like `STRUCT(hour BIGINT, user_id INTEGER)` —
    // no `..r` appears.
    assert!(
        !type_str.contains("..r"),
        "resolved type should not show `..r`; got `{type_str}`"
    );
}

// ─── Test 4: expansion emits explicit field references ────────────────────────

#[test]
fn expansion_emits_explicit_field_references() {
    // TDD test 4: the `..event` spread in `{EXTRACT(HOUR FROM event.ts) AS hour, ..event}`
    // must expand to `event.user_id AS user_id` when the row var `r` is bound
    // to `[(user_id, Integer)]`.
    //
    // Tested via the pure `expand_brace_struct_body` function which takes:
    //   - The raw body text (the brace-struct literal source)
    //   - The spread param name (`event`)
    //   - The extras from the row var binding
    // And returns the expanded SQL string.
    use smelt_db::function_body_check::expand_brace_struct_body;
    use smelt_types::DataType;

    let body_text = "{EXTRACT(HOUR FROM event.ts) AS hour, ..event}";
    let extras = vec![("user_id".to_string(), DataType::Integer)];
    let expanded = expand_brace_struct_body(body_text, "event", &extras);

    assert!(
        expanded.contains("event.user_id AS user_id"),
        "expanded body should contain `event.user_id AS user_id`; got `{expanded}`"
    );
    // The spread placeholder `..event` should no longer appear.
    assert!(
        !expanded.contains("..event"),
        "expanded body should not contain `..event`; got `{expanded}`"
    );
    // The original non-spread field should still be present.
    assert!(
        expanded.contains("EXTRACT(HOUR FROM event.ts) AS hour"),
        "expanded body should preserve non-spread fields; got `{expanded}`"
    );
}
