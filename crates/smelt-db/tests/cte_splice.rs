//! Phase 20 — CTE schema extraction + splice-point context inference.
//!
//! Tests:
//!   1. `cte_schema_extracted_from_body` — body `WITH s AS (SELECT …)` extracts
//!      the CTE's schema into the returned TypeContext.
//!   2. `context_inferred_from_splice_point` — `filters` in `WHERE filters` after
//!      `FROM source` → inferred context = source's columns.
//!   3. `context_matches_explicit_annotation` — positive: `Expr<Boolean, source>`
//!      spliced after `FROM source` → no ContextMismatch; negative: wrong ctx_name
//!      → ContextMismatch.
//!   4. `intersection_over_multi_splice` — `pred` in `WHERE` (scope {id,name,amount})
//!      and `HAVING` (scope {id,total}) → inferred context = {id}.
//!   5. `cte_cycle_detected` — `WITH a AS (…FROM b…), b AS (…FROM a…)` → CteCycle.

use std::path::PathBuf;

use smelt_db::{
    extract_function_body_cte_schemas, file_diagnostics, infer_splice_contexts, Database,
    DiagnosticCode, SourceFile, TypeContext, TypedColumn, Workspace,
};
use smelt_parser::File as AstFile;
use smelt_types::DataType;

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(
        project_root.clone(),
        "version: 1\nsources: []\n".to_string(),
    );
    let mut handles = Vec::new();
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn get_body_select(db: &Database, file: SourceFile) -> smelt_parser::ast::SelectStmt {
    let parse = smelt_db::parse_file(db, file);
    let syntax = parse.syntax();
    let ast_file = AstFile::cast(syntax).expect("valid AstFile");
    let define = ast_file.defines().next().expect("at least one define");
    let body = define.body().expect("define has a body");
    body.select_stmt().expect("body is a SELECT statement")
}

#[test]
fn cte_schema_extracted_from_body() {
    // Phase 20 TDD test 1: body WITH s AS (SELECT id, name FROM source) extracts
    // schema {id, name} for CTE 's' into the returned TypeContext.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("cte_fn.sql");
    let sql = "smelt.define cte_fn( \
       source: TableExpr \
     ) -> TableExpr AS ( \
       WITH s AS (SELECT id, name FROM source) \
       SELECT * FROM s \
     )\n";

    let (db, _ws, files) = build_db(root, &[(fn_path, sql)]);

    let mut seed_ctx = TypeContext::new();
    seed_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            ("name".to_string(), TypedColumn::nullable(DataType::Text)),
        ],
    );

    let select = get_body_select(&db, files[0]);
    let text = smelt_parser::strip_frontmatter(files[0].text(&db));
    let no_smelt_fn =
        |_call: &smelt_parser::ast::SmeltFnCall| -> Option<Vec<(String, TypedColumn)>> { None };
    let (ctx, cycle_diags) =
        extract_function_body_cte_schemas(&select, &seed_ctx, &text, &no_smelt_fn);

    assert!(
        cycle_diags.is_empty(),
        "no cycles expected; got {cycle_diags:#?}"
    );
    assert!(ctx.is_cte("s"), "CTE 's' should be registered in context");
    let s_cols = ctx.cte_columns("s");
    let col_names: Vec<&str> = s_cols.iter().map(|(n, _)| *n).collect();
    assert!(col_names.contains(&"id"), "CTE 's' should have column 'id'");
    assert!(
        col_names.contains(&"name"),
        "CTE 's' should have column 'name'"
    );
}

#[test]
fn context_inferred_from_splice_point() {
    // Phase 20 TDD test 2: filters is referenced in WHERE after FROM source;
    // the inferred splice context = source's columns {id, name, amount}.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("filter_fn.sql");
    let sql = "smelt.define filter_fn( \
       source: TableExpr, \
       filters: Expr<Boolean> \
     ) -> TableExpr AS ( \
       SELECT * FROM source WHERE filters \
     )\n";

    let (db, _ws, files) = build_db(root, &[(fn_path, sql)]);

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            ("name".to_string(), TypedColumn::nullable(DataType::Text)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param("filters", TypedColumn::nullable(DataType::Boolean));

    let sigs = smelt_db::file_signature_inputs(&db, files[0]);
    let sig = sigs
        .iter()
        .find(|s| s.name == "filter_fn")
        .expect("filter_fn sig");

    let select = get_body_select(&db, files[0]);
    let inferred = infer_splice_contexts(sig, &select, &body_ctx);

    let ctx = inferred
        .get("filters")
        .expect("filters should have an inferred splice context");
    let col_names: Vec<&str> = ctx.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        col_names.contains(&"id"),
        "id should be in inferred context"
    );
    assert!(
        col_names.contains(&"name"),
        "name should be in inferred context"
    );
    assert!(
        col_names.contains(&"amount"),
        "amount should be in inferred context"
    );
}

#[test]
fn context_matches_explicit_annotation() {
    // Phase 20 TDD test 3.
    // Positive case: filters: Expr<Boolean, source> spliced after FROM source → no ContextMismatch.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("filter_ann.sql");
    let sql_ok = "smelt.define filter_ann( \
       source: TableExpr, \
       filters: Expr<Boolean, source> \
     ) -> TableExpr AS ( \
       SELECT * FROM source WHERE filters \
     )\n";

    let (db, ws, files) = build_db(root.clone(), &[(fn_path, sql_ok)]);
    let diags = file_diagnostics(&db, ws, files[0]);
    let mismatch: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContextMismatch))
        .collect();
    assert!(
        mismatch.is_empty(),
        "no ContextMismatch when annotation matches splice point; got {mismatch:#?}"
    );

    // Negative case: filters: Expr<Boolean, source2> but body splices filters after
    // FROM source1 → ContextMismatch.
    let fn_path2 = root.join("functions").join("filter_mismatch.sql");
    let sql_bad = "smelt.define filter_mismatch( \
       source1: TableExpr, \
       source2: TableExpr, \
       filters: Expr<Boolean, source2> \
     ) -> TableExpr AS ( \
       SELECT * FROM source1 WHERE filters \
     )\n";

    let (db2, ws2, files2) = build_db(root, &[(fn_path2, sql_bad)]);
    let diags2 = file_diagnostics(&db2, ws2, files2[0]);
    let mismatch2: Vec<_> = diags2
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContextMismatch))
        .collect();
    assert!(
        !mismatch2.is_empty(),
        "should emit ContextMismatch when annotation doesn't match splice; got {diags2:#?}"
    );
    assert!(
        mismatch2[0].message.contains("source2") || mismatch2[0].message.contains("source1"),
        "message should mention the mismatched context names; got {:?}",
        mismatch2[0].message
    );
}

#[test]
fn intersection_over_multi_splice() {
    // Phase 20 TDD test 4: pred appears in WHERE (scope {id,name,amount}) and
    // HAVING (scope {id,total}); inferred context = intersection = {id}.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("multi_splice.sql");
    let sql = "smelt.define multi_splice( \
       source: TableExpr, \
       pred: Expr<Boolean> \
     ) -> TableExpr AS ( \
       SELECT id, SUM(amount) AS total FROM source WHERE pred GROUP BY id HAVING pred \
     )\n";

    let (db, _ws, files) = build_db(root, &[(fn_path, sql)]);

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            ("name".to_string(), TypedColumn::nullable(DataType::Text)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param("pred", TypedColumn::nullable(DataType::Boolean));

    let sigs = smelt_db::file_signature_inputs(&db, files[0]);
    let sig = sigs
        .iter()
        .find(|s| s.name == "multi_splice")
        .expect("multi_splice sig");

    let select = get_body_select(&db, files[0]);
    let inferred = infer_splice_contexts(sig, &select, &body_ctx);

    let ctx = inferred
        .get("pred")
        .expect("pred should have an inferred splice context");
    let col_names: Vec<&str> = ctx.iter().map(|(n, _)| n.as_str()).collect();
    assert!(col_names.contains(&"id"), "id should be in intersection");
    assert!(
        !col_names.contains(&"name"),
        "name should NOT be in intersection (WHERE-only)"
    );
    assert!(
        !col_names.contains(&"amount"),
        "amount should NOT be in intersection (WHERE-only)"
    );
    assert!(
        !col_names.contains(&"total"),
        "total should NOT be in intersection (HAVING-only)"
    );
}

#[test]
fn cte_cycle_detected() {
    // Phase 20 TDD test 5: WITH a AS (SELECT * FROM b), b AS (SELECT * FROM a)
    // forms a mutual cycle → CteCycle diagnostic.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("broken").join("models").join("fn_cte_cycle.sql");
    let sql = "smelt.define bad_cte( \
       source: TableExpr \
     ) -> TableExpr AS ( \
       WITH a AS (SELECT * FROM b), b AS (SELECT * FROM a) \
       SELECT * FROM a \
     )\n";

    let (db, ws, files) = build_db(root, &[(fn_path, sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);
    let cycle_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CteCycle))
        .collect();
    assert!(
        !cycle_diags.is_empty(),
        "should emit CteCycle for mutually-recursive CTEs; got {diags:#?}"
    );
}
