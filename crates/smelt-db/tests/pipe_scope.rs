//! Phase 3 TDD tests: stage-to-stage column scope threading for pipe queries.
//!
//! Tests that EXTEND/SET/DROP/RENAME pipe stages correctly propagate scope,
//! and that undeclared-column diagnostics are emitted when a later stage
//! references a column dropped or never introduced.

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

/// Collect all diagnostics (file + type) for a given source file.
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

// ── Test 1: extend_column_visible_next_stage ─────────────────────────────────

/// A column introduced by EXTEND must be visible to the immediately following
/// WHERE stage. No UndeclaredColumn diagnostic should fire for `s`.
///
/// Pipe query: `FROM t |> EXTEND a + b AS s |> WHERE s > 0`
/// Initial table t(a INTEGER, b INTEGER) provides `a` and `b`;
/// EXTEND adds `s`; WHERE references `s` — must be in scope.
#[test]
fn extend_column_visible_next_stage() {
    let root = PathBuf::from("/fake/pipe_scope1");
    let t_path = root.join("models").join("t.sql");
    let pipe_path = root.join("models").join("pipe_extend.sql");

    // Model t with two integer columns
    let t_sql = "SELECT CAST(NULL AS INTEGER) AS a, CAST(NULL AS INTEGER) AS b\n";
    // Pipe query using EXTEND; s is introduced and immediately referenced
    let pipe_sql = "FROM smelt.models.t |> EXTEND a + b AS s |> WHERE s > 0\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(t_path, t_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared_col_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .collect();

    assert!(
        undeclared_col_diags.is_empty(),
        "Expected no UndeclaredColumn diagnostics when EXTEND introduces `s` and WHERE uses it; got: {undeclared_col_diags:#?}"
    );
}

// ── Test 2: drop_then_reference_errors ───────────────────────────────────────

/// After `|> DROP a`, column `a` must no longer be in scope.
/// A `|> SELECT a` following the DROP must produce exactly one
/// UndeclaredColumn diagnostic for `a`.
#[test]
fn drop_then_reference_errors() {
    let root = PathBuf::from("/fake/pipe_scope2");
    let t_path = root.join("models").join("t2.sql");
    let pipe_path = root.join("models").join("pipe_drop.sql");

    let t_sql = "SELECT CAST(NULL AS INTEGER) AS a, CAST(NULL AS INTEGER) AS b\n";
    // DROP removes `a`; the following SELECT references it — should be an error
    let pipe_sql = "FROM smelt.models.t2 |> DROP a |> SELECT a\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(t_path, t_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared_for_a: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
                && d.message.contains('a')
        })
        .collect();

    assert_eq!(
        undeclared_for_a.len(),
        1,
        "Expected exactly one UndeclaredColumn for `a` after DROP; got: {diags:#?}"
    );
}

// ── Test 3: rename_and_set_schema ─────────────────────────────────────────────

/// After `|> RENAME a AS x`, the output schema has `x` not `a`.
/// A following stage that references `x` must succeed; one that references `a`
/// would fail (not tested here — schema tracking suffices for this assertion).
#[test]
fn rename_stays_in_scope() {
    let root = PathBuf::from("/fake/pipe_scope3");
    let t_path = root.join("models").join("t3.sql");
    let pipe_path = root.join("models").join("pipe_rename.sql");

    let t_sql = "SELECT CAST(NULL AS INTEGER) AS a, CAST(NULL AS INTEGER) AS b\n";
    // Rename a → x, then reference x in WHERE — must succeed
    let pipe_sql = "FROM smelt.models.t3 |> RENAME a AS x |> WHERE x > 0\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(t_path, t_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "Expected no UndeclaredColumn after RENAME a AS x and WHERE x > 0; got: {undeclared:#?}"
    );
}

/// After `|> SET a = a * 2`, `a` stays in scope (SET replaces in-place).
/// A following stage referencing `a` must succeed.
#[test]
fn set_column_stays_in_scope() {
    let root = PathBuf::from("/fake/pipe_scope4");
    let t_path = root.join("models").join("t4.sql");
    let pipe_path = root.join("models").join("pipe_set.sql");

    let t_sql = "SELECT CAST(NULL AS INTEGER) AS a, CAST(NULL AS INTEGER) AS b\n";
    // SET replaces a's value but keeps it in scope
    let pipe_sql = "FROM smelt.models.t4 |> SET a = a * 2 |> WHERE a > 0\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(t_path, t_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "Expected no UndeclaredColumn after SET a = a * 2 and WHERE a > 0; got: {undeclared:#?}"
    );
}

// ── Test: aggregate_collapses_scope ─────────────────────────────────────────

/// After `AGGREGATE sum(amount) AS rev GROUP BY cust_id`:
///
/// - `cust_id` and `rev` are in scope
/// - `amount` (pre-aggregation column) is NOT in scope
///
/// A reference to `amount` in a post-AGGREGATE WHERE must raise UndeclaredColumn.
#[test]
fn aggregate_collapses_scope() {
    let root = PathBuf::from("/fake/pipe_agg1");
    let orders_path = root.join("models").join("orders.sql");
    let pipe_path = root.join("models").join("pipe_agg.sql");

    // orders has cust_id and amount
    let orders_sql = "SELECT CAST(NULL AS INTEGER) AS cust_id, CAST(NULL AS DOUBLE) AS amount\n";
    // After AGGREGATE, `amount` should no longer be in scope
    let pipe_sql =
        "FROM smelt.models.orders |> AGGREGATE sum(amount) AS rev GROUP BY cust_id |> WHERE amount > 10\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(orders_path, orders_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared_amount: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
                && d.message.to_lowercase().contains("amount")
        })
        .collect();

    assert_eq!(
        undeclared_amount.len(),
        1,
        "Expected exactly one UndeclaredColumn for `amount` after AGGREGATE; got: {diags:#?}"
    );
}

// ── Test: aggregate_output_in_scope ─────────────────────────────────────────

/// After `AGGREGATE sum(amount) AS rev GROUP BY cust_id`:
///
/// - both `cust_id` and `rev` must be accessible in a following stage.
///
/// A `|> SELECT cust_id, rev` must produce no UndeclaredColumn diagnostics.
#[test]
fn aggregate_output_in_scope() {
    let root = PathBuf::from("/fake/pipe_agg2");
    let orders_path = root.join("models").join("orders2.sql");
    let pipe_path = root.join("models").join("pipe_agg2.sql");

    let orders_sql = "SELECT CAST(NULL AS INTEGER) AS cust_id, CAST(NULL AS DOUBLE) AS amount\n";
    let pipe_sql =
        "FROM smelt.models.orders2 |> AGGREGATE sum(amount) AS rev GROUP BY cust_id |> SELECT cust_id, rev\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(orders_path, orders_sql), (pipe_path.clone(), pipe_sql)],
    );
    let pipe_file = files[1];

    let diags = all_diagnostics(&db, ws, pipe_file);
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "Expected no UndeclaredColumn after AGGREGATE when referencing cust_id and rev; got: {undeclared:#?}"
    );
}
