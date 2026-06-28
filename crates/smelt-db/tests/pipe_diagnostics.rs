//! Tests for deferred-operator errors and stage-boundary disambiguation in pipe SQL.
//!
//! Tests that:
//! 1. Deferred pipe operators (PIVOT/UNPIVOT/WINDOW/CALL/TABLESAMPLE/ASSERT) emit
//!    `PipeOperatorUnsupported` — not a silent no-op.
//! 2. A stage-boundary `|>` in a FROM-first body does NOT raise `PipeInDataPosition`.
//! 3. A `|>` inside a `WHERE` predicate expression still raises `PipeInDataPosition`.
//! 4. Meta-expression pipes in a SELECT item position are unaffected by the
//!    `PipeInDataPosition` check (SELECT item is not a data-world position).

use std::path::PathBuf;

use smelt_db::{
    check_type_diagnostics, file_diagnostics, Database, DiagnosticAcc, DiagnosticCode,
    DiagnosticSeverity, SourceFile, Workspace,
};

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

/// Collect all diagnostics (file + type check) for a source file.
fn all_diagnostics(db: &Database, ws: Workspace, file: SourceFile) -> Vec<smelt_db::Diagnostic> {
    let file_diags = file_diagnostics(db, ws, file);
    let type_diags: Vec<_> = check_type_diagnostics::accumulated::<DiagnosticAcc>(db, ws, file)
        .into_iter()
        .map(|d| d.0.clone())
        .collect();
    let mut all: Vec<_> = file_diags.into_iter().chain(type_diags).collect();
    all.sort_by_key(|d| d.range.start());
    all
}

// ── Test 1: deferred_operators_fail_loud ─────────────────────────────────────

/// Each deferred operator used as a pipe stage must produce a
/// `PipeOperatorUnsupported` diagnostic, never a silent no-op.
///
/// Spec §Surface "Deferred operators", §Constraints 4.
#[test]
fn deferred_operators_fail_loud() {
    let root = PathBuf::from("/fake/pipe_diagnostics_deferred");

    // Each deferred operator with its expected reason substring.
    let cases: &[(&str, &str, &str)] = &[
        (
            "pivot",
            "FROM t |> PIVOT(SUM(v) FOR k IN ('a', 'b'))",
            "output columns depend on data values",
        ),
        (
            "unpivot",
            "FROM t |> UNPIVOT(v FOR k IN (a, b))",
            "output columns depend on data values",
        ),
        (
            "window",
            "FROM t |> WINDOW w AS (PARTITION BY k ORDER BY v)",
            "WINDOW operator form is not supported",
        ),
        (
            "call",
            "FROM t |> CALL my_tvf()",
            "table-valued function piping has no end-to-end smelt support",
        ),
        (
            "tablesample",
            "FROM t |> TABLESAMPLE BERNOULLI(10)",
            "sampling is available on a FROM table reference only",
        ),
        (
            "assert",
            "FROM t |> ASSERT v > 0",
            "row-level runtime assertions have no smelt equivalent construct",
        ),
    ];

    // Provide a base table model `t` for all tests.
    let t_path = root.join("models").join("t.sql");
    let t_sql = "SELECT CAST(NULL AS INTEGER) AS k, CAST(NULL AS INTEGER) AS v\n";

    for (op_name, pipe_sql, expected_reason_substr) in cases {
        let pipe_path = root.join("models").join(format!("pipe_{}.sql", op_name));
        let (db, ws, files) = build_db(
            root.clone(),
            "version: 1\nsources: []\n",
            &[(t_path.clone(), t_sql), (pipe_path.clone(), pipe_sql)],
        );
        let pipe_file = files[1];

        let diags = all_diagnostics(&db, ws, pipe_file);

        let unsupported_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::PipeOperatorUnsupported))
            .collect();

        assert!(
            !unsupported_diags.is_empty(),
            "operator {op_name}: expected PipeOperatorUnsupported, got none; all diags: {diags:#?}"
        );

        // Verify the message includes the operator name and reason.
        let op_upper = op_name.to_ascii_uppercase();
        for d in &unsupported_diags {
            assert!(
                d.message.contains(&op_upper),
                "operator {op_name}: message should contain '{op_upper}'; got: {:?}",
                d.message
            );
            assert!(
                d.message.contains(expected_reason_substr),
                "operator {op_name}: message should contain reason '{expected_reason_substr}'; got: {:?}",
                d.message
            );
            assert_eq!(
                d.severity,
                DiagnosticSeverity::Error,
                "operator {op_name}: severity must be Error"
            );
        }
    }
}

// ── Test 2: pipe_at_stage_boundary_is_not_data_position ─────────────────────

/// A stage-boundary `|>` in a FROM-first body must NOT raise `PipeInDataPosition`.
///
/// Spec §Semantics "Token disambiguation" rule 1.
#[test]
fn pipe_at_stage_boundary_is_not_data_position() {
    let root = PathBuf::from("/fake/pipe_diagnostics_boundary");
    let t_path = root.join("models").join("t.sql");
    let t_sql = "SELECT CAST(NULL AS INTEGER) AS a, CAST(NULL AS INTEGER) AS b\n";

    // Valid pipe query: each `|>` is a stage boundary, not a data-world expression.
    let pipe_path = root.join("models").join("pipe_boundary.sql");
    let pipe_sql = "FROM smelt.models.t |> WHERE a > 0 |> SELECT b\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(t_path, t_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);

    let data_pos_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::PipeInDataPosition))
        .collect();

    assert!(
        data_pos_diags.is_empty(),
        "expected no PipeInDataPosition for stage-boundary |>; got: {data_pos_diags:#?}"
    );
}

// ── Test 3: pipe_in_where_expression_still_rejected ─────────────────────────

/// A `|>` used inside a WHERE predicate expression (Data-World position) must
/// still raise `PipeInDataPosition`.
///
/// Spec §Semantics "Token disambiguation" rule 2.
#[test]
fn pipe_in_where_expression_still_rejected() {
    let root = PathBuf::from("/fake/pipe_diagnostics_where_expr");

    // A standard SQL model (not a pipe query) with |> inside the WHERE clause.
    // The parser will produce a PIPE_EXPR node inside a WHERE_CLAUSE.
    let model_path = root.join("models").join("pipe_where.sql");
    // `x |> some_fn()` inside WHERE — data-world position.
    let model_sql = "SELECT a FROM t WHERE a |> some_fn()\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(model_path.clone(), model_sql)],
    );
    let model_file = files[0];

    let diags = all_diagnostics(&db, ws, model_file);

    let data_pos_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::PipeInDataPosition))
        .collect();

    assert!(
        !data_pos_diags.is_empty(),
        "expected PipeInDataPosition for |> inside WHERE expression; got none; all diags: {diags:#?}"
    );
}

// ── Test 4: meta_pipe_unaffected ─────────────────────────────────────────────

/// A `|>` in a SELECT item position must NOT emit `PipeInDataPosition`.
///
/// SELECT item is not a data-world position: the ancestor walk from the PIPE_EXPR
/// reaches SELECT_STMT (a stop point) before encountering WHERE_CLAUSE or PIPE_STAGE.
/// This confirms that narrowing `PipeInDataPosition` to data-world expression positions
/// does not affect meta-expression pipe behaviour.
///
/// Spec §Constraints 5: narrowing `PipeInDataPosition` must not change
/// meta-expression pipe behaviour.
#[test]
fn meta_pipe_unaffected() {
    let root = PathBuf::from("/fake/pipe_diagnostics_meta");

    // A model that uses the meta |> operator in a SELECT item position.
    // This is NOT a data-world position (not inside WHERE_CLAUSE or PIPE_STAGE),
    // so PipeInDataPosition must NOT fire even though |> appears in SQL.
    //
    // Spec §Constraints 5: narrowing PipeInDataPosition to data-world expression
    // positions must not change meta-expression pipe behaviour.
    let model_path = root.join("models").join("pipe_meta.sql");
    // |> in SELECT item position — meta-context pipe, not a data-world position.
    // `a |> some_fn()` creates a PIPE_EXPR; it is in a SELECT item, not a WHERE clause.
    let model_sql = "SELECT a |> some_fn() FROM t\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(model_path.clone(), model_sql)],
    );
    let model_file = files[0];

    let diags = all_diagnostics(&db, ws, model_file);

    // PipeInDataPosition must NOT fire — SELECT item is not a data-world position.
    let data_pos_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::PipeInDataPosition))
        .collect();

    assert!(
        data_pos_diags.is_empty(),
        "meta |> in SELECT item must not emit PipeInDataPosition; got: {data_pos_diags:#?}"
    );
}
