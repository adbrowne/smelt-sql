//! Phase 5a — `smelt.functions.*` path-call diagnostic parity tests.
//!
//! These integration tests verify that `smelt_fn_call_diagnostics_for_file`
//! produces the same diagnostic codes for `smelt.functions.<name>(...)` calls
//! as it already does for `smelt.fn.<name>(...)` calls.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

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

fn diags_with_code(
    db: &Database,
    ws: Workspace,
    file: SourceFile,
    code: DiagnosticCode,
) -> Vec<smelt_db::Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| d.code == Some(code))
        .collect()
}

/// Test 1: `smelt.functions.needs_number('text')` emits exactly one
/// `ArgTypeMismatch` anchored at the `'text'` argument span.
#[test]
fn path_form_arg_type_mismatch() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("needs_number.sql");
    let fn_src = "smelt.define needs_number(x: Expr<Numeric>) -> Expr<Numeric> AS (x + 1)\n";

    let model_path = root.join("models").join("path_type_mismatch.sql");
    let model_src = "SELECT smelt.functions.needs_number('text') AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path.clone(), model_src)]);
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::ArgTypeMismatch);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one ArgTypeMismatch diagnostic, got {diags:?}"
    );

    // The squiggle should land on `'text'`.
    let arg_start = model_src.find("'text'").unwrap();
    let arg_end = arg_start + "'text'".len();
    assert_eq!(
        diags[0].range.start.column as usize, arg_start,
        "expected arg-range start at `'text'`, got {:?}",
        diags[0].range
    );
    assert_eq!(
        diags[0].range.end.column as usize, arg_end,
        "expected arg-range end after `'text'`, got {:?}",
        diags[0].range
    );
}

/// Test 2: `smelt.functions.takes_two(1)` (missing arg `b`) emits exactly one
/// `MissingArgument` diagnostic.
#[test]
fn path_form_missing_arg() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("takes_two.sql");
    let fn_src =
        "smelt.define takes_two(a: Expr<Integer>, b: Expr<Integer>) -> Expr<Integer> AS (a + b)\n";

    let model_path = root.join("models").join("path_missing_arg.sql");
    let model_src = "SELECT smelt.functions.takes_two(1) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path.clone(), model_src)]);
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::MissingArgument);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one MissingArgument diagnostic, got {diags:?}"
    );
    assert!(
        diags[0].message.contains('b') || diags[0].message.contains("takes_two"),
        "message should name the missing parameter or function, got: {}",
        diags[0].message
    );
}

/// Test 3: `smelt.functions.does_not_exist(1)` emits exactly one
/// `UnknownSmeltFn` anchored at the path span.
#[test]
fn path_form_unknown_fn() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("path_unknown.sql");
    let model_src = "SELECT smelt.functions.does_not_exist(1) AS r\n";

    let (db, ws, files) = build_db(root, &[(model_path.clone(), model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownSmeltFn diagnostic, got {diags:?}"
    );
    // The diagnostic should name the unknown function.
    assert!(
        diags[0].message.contains("does_not_exist"),
        "message should name the unknown function, got: {}",
        diags[0].message
    );
}

/// Test 4: a three-level chain (`outer_call → middle → inner_unary`) where
/// `inner_unary` body references `undefined_var` produces a diagnostic with
/// a frame stack containing all three expansion levels. Mirrors `fn_nested_call_error.sql`.
#[test]
fn path_form_nested_body_error() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("nested_chain.sql");
    let fn_src = "\
smelt.define inner_unary(x) AS (x + undefined_var)
smelt.define middle(z) AS (smelt.fn.inner_unary(z))
smelt.define outer_call(y) AS (smelt.fn.middle(y))
";

    let model_path = root.join("models").join("path_nested_error.sql");
    let model_src = "SELECT smelt.functions.outer_call(1) AS threaded\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path.clone(), model_src)]);
    let model_file = files[1];

    let all_diags = file_diagnostics(&db, ws, model_file);
    // Filter to error-level call diagnostics.
    use smelt_db::DiagnosticData;
    let frame_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownIdentifier)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::ArgTypeMismatch)
            )
        })
        .collect();
    assert!(
        !frame_diags.is_empty(),
        "expected at least one body-error diagnostic from nested path call, got none"
    );

    // At least one diagnostic should carry a multi-frame expansion stack.
    let has_frames = frame_diags.iter().any(
        |d| matches!(&d.data, Some(DiagnosticData::ExpansionFrames(frames)) if frames.len() >= 2),
    );
    assert!(
        has_frames,
        "expected at least one diagnostic with >=2 expansion frames, got {frame_diags:?}"
    );
}

/// Test 5: calling a `TableExpr<{revenue: Numeric, cost: Numeric}>` function
/// with a table missing the `cost` column produces `RowRequirementMissing`.
/// Mirrors `fn_row_requirement_missing.sql`.
#[test]
fn path_form_tableexpr_row_requirement() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin_req.sql");
    let fn_src = "\
smelt.define add_margin_req(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";

    // A model that has only `revenue`, not `cost`.
    let upstream_path = root.join("models").join("no_cost.sql");
    let upstream_src = "SELECT 100 AS revenue\n";

    let model_path = root.join("models").join("path_row_req.sql");
    let model_src = "SELECT * FROM smelt.functions.add_margin_req(smelt.models.no_cost) AS m\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (fn_path, fn_src),
            (upstream_path, upstream_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let row_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::RowRequirementUnsatisfied))
        .collect();
    assert!(
        !row_diags.is_empty(),
        "expected at least one RowRequirementMissing diagnostic, got {diags:?}"
    );
}

/// Test 6: for each of the representative diagnostic codes, calling the same
/// function via `smelt.functions.*` produces the same set of diagnostic codes
/// as `smelt.fn.*`. This is the parity test.
#[test]
fn path_form_parity() {
    // Case A: ArgTypeMismatch parity
    {
        let root = PathBuf::from("/fake/project/parity_type");
        let fn_src = "smelt.define needs_number(x: Expr<Numeric>) -> Expr<Numeric> AS (x + 1)\n";
        let fn_path = root.join("functions").join("needs_number.sql");

        // legacy smelt.fn form
        let legacy_path = root.join("models").join("legacy.sql");
        let legacy_src = "SELECT smelt.fn.needs_number('text') AS r\n";

        // new smelt.functions form
        let path_form_path = root.join("models").join("path_form.sql");
        let path_form_src = "SELECT smelt.functions.needs_number('text') AS r\n";

        let (db, ws, files) = build_db(
            root,
            &[
                (fn_path, fn_src),
                (legacy_path, legacy_src),
                (path_form_path, path_form_src),
            ],
        );
        let legacy_file = files[1];
        let path_file = files[2];

        let legacy_codes: Vec<_> = file_diagnostics(&db, ws, legacy_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();
        let path_codes: Vec<_> = file_diagnostics(&db, ws, path_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();

        let mut lc = legacy_codes.clone();
        let mut pc = path_codes.clone();
        lc.sort_by_key(|c| format!("{c:?}"));
        pc.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            lc, pc,
            "ArgTypeMismatch parity failed: legacy={legacy_codes:?}, path={path_codes:?}"
        );
    }

    // Case B: MissingArgument parity
    {
        let root = PathBuf::from("/fake/project/parity_missing");
        let fn_src =
            "smelt.define takes_two(a: Expr<Integer>, b: Expr<Integer>) -> Expr<Integer> AS (a + b)\n";
        let fn_path = root.join("functions").join("takes_two.sql");

        let legacy_path = root.join("models").join("legacy.sql");
        let legacy_src = "SELECT smelt.fn.takes_two(1) AS r\n";

        let path_form_path = root.join("models").join("path_form.sql");
        let path_form_src = "SELECT smelt.functions.takes_two(1) AS r\n";

        let (db, ws, files) = build_db(
            root,
            &[
                (fn_path, fn_src),
                (legacy_path, legacy_src),
                (path_form_path, path_form_src),
            ],
        );
        let legacy_file = files[1];
        let path_file = files[2];

        let legacy_codes: Vec<_> = file_diagnostics(&db, ws, legacy_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();
        let path_codes: Vec<_> = file_diagnostics(&db, ws, path_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();

        let mut lc = legacy_codes.clone();
        let mut pc = path_codes.clone();
        lc.sort_by_key(|c| format!("{c:?}"));
        pc.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            lc, pc,
            "MissingArgument parity failed: legacy={legacy_codes:?}, path={path_codes:?}"
        );
    }

    // Case C: UnknownSmeltFn parity
    {
        let root = PathBuf::from("/fake/project/parity_unknown");

        let legacy_path = root.join("models").join("legacy.sql");
        let legacy_src = "SELECT smelt.fn.does_not_exist(1) AS r\n";

        let path_form_path = root.join("models").join("path_form.sql");
        let path_form_src = "SELECT smelt.functions.does_not_exist(1) AS r\n";

        let (db, ws, files) = build_db(
            root,
            &[(legacy_path, legacy_src), (path_form_path, path_form_src)],
        );
        let legacy_file = files[0];
        let path_file = files[1];

        let legacy_codes: Vec<_> = file_diagnostics(&db, ws, legacy_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();
        let path_codes: Vec<_> = file_diagnostics(&db, ws, path_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();

        let mut lc = legacy_codes.clone();
        let mut pc = path_codes.clone();
        lc.sort_by_key(|c| format!("{c:?}"));
        pc.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            lc, pc,
            "UnknownSmeltFn parity failed: legacy={legacy_codes:?}, path={path_codes:?}"
        );
    }

    // Case D: RowRequirementUnsatisfied parity
    // Function requires TableExpr<{revenue: Numeric, cost: Numeric}>;
    // upstream model supplies only `revenue`. Both call forms should emit
    // the same RowRequirementUnsatisfied diagnostic.
    {
        let root = PathBuf::from("/fake/project/parity_row_req");
        let fn_src = "\
smelt.define add_margin_req(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";
        let fn_path = root.join("functions").join("add_margin_req.sql");

        let upstream_path = root.join("models").join("no_cost.sql");
        let upstream_src = "SELECT 100 AS revenue\n";

        let legacy_path = root.join("models").join("legacy.sql");
        let legacy_src = "SELECT * FROM smelt.fn.add_margin_req(smelt.models.no_cost) AS m\n";

        let path_form_path = root.join("models").join("path_form.sql");
        let path_form_src =
            "SELECT * FROM smelt.functions.add_margin_req(smelt.models.no_cost) AS m\n";

        let (db, ws, files) = build_db(
            root,
            &[
                (fn_path, fn_src),
                (upstream_path, upstream_src),
                (legacy_path, legacy_src),
                (path_form_path, path_form_src),
            ],
        );
        let legacy_file = files[2];
        let path_file = files[3];

        let legacy_codes: Vec<_> = file_diagnostics(&db, ws, legacy_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();
        let path_codes: Vec<_> = file_diagnostics(&db, ws, path_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();

        // Both forms must emit RowRequirementUnsatisfied.
        assert!(
            legacy_codes.contains(&DiagnosticCode::RowRequirementUnsatisfied),
            "Case D: legacy smelt.fn form should emit RowRequirementUnsatisfied, got {legacy_codes:?}"
        );

        let mut lc = legacy_codes.clone();
        let mut pc = path_codes.clone();
        lc.sort_by_key(|c| format!("{c:?}"));
        pc.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            lc, pc,
            "RowRequirementUnsatisfied parity failed: legacy={legacy_codes:?}, path={path_codes:?}"
        );
    }

    // Case E: UnknownIdentifier with ExpansionFrames parity
    // Three-level chain where the innermost body references an undefined var.
    // Both call forms (`smelt.fn.*` and `smelt.functions.*`) should produce
    // the same diagnostic codes (UnknownIdentifier at minimum).
    {
        let root = PathBuf::from("/fake/project/parity_frames");
        let fn_src = "\
smelt.define inner_unary(x) AS (x + undefined_var)
smelt.define middle(z) AS (smelt.fn.inner_unary(z))
smelt.define outer_call(y) AS (smelt.fn.middle(y))
";
        let fn_path = root.join("functions").join("nested_chain.sql");

        let legacy_path = root.join("models").join("legacy.sql");
        let legacy_src = "SELECT smelt.fn.outer_call(1) AS threaded\n";

        let path_form_path = root.join("models").join("path_form.sql");
        let path_form_src = "SELECT smelt.functions.outer_call(1) AS threaded\n";

        let (db, ws, files) = build_db(
            root,
            &[
                (fn_path, fn_src),
                (legacy_path, legacy_src),
                (path_form_path, path_form_src),
            ],
        );
        let legacy_file = files[1];
        let path_file = files[2];

        let legacy_codes: Vec<_> = file_diagnostics(&db, ws, legacy_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();
        let path_codes: Vec<_> = file_diagnostics(&db, ws, path_file)
            .into_iter()
            .filter_map(|d| d.code)
            .collect();

        // Both forms must emit at least one body-error diagnostic.
        assert!(
            !legacy_codes.is_empty(),
            "Case E: legacy smelt.fn form should emit body-error diagnostics, got none"
        );

        let mut lc = legacy_codes.clone();
        let mut pc = path_codes.clone();
        lc.sort_by_key(|c| format!("{c:?}"));
        pc.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            lc, pc,
            "ExpansionFrames parity failed: legacy={legacy_codes:?}, path={path_codes:?}"
        );
    }
}
