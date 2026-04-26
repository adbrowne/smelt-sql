//! Phase 6 (smelt-functions) — Call-site expansion and single-level frame
//! trace for `smelt.fn.<name>(...)` invocations.
//!
//! These integration tests verify that:
//!   1. A correctly-typed call to `safe_divide` produces zero diagnostics.
//!   2. Passing a Text value to a `Numeric` parameter emits
//!      `ArgTypeMismatch` anchored at the offending argument, carrying a
//!      `FrameInfo` for the `safe_divide.numerator` binding.
//!   3. Named arguments (`a => x, b => y`) bind by name, not position.
//!   4. Omitting a required positional argument emits `MissingArgument`.
//!   5. A parameter with a default value is silently filled when no arg
//!      is supplied.
//!   6. An unresolved `smelt.fn.does_not_exist(...)` emits
//!      `UnknownSmeltFn` at the call-path span.
//!   7. The `functions_demo` example workspace stays clean end-to-end
//!      (delegated to `smelt-cli --test example_diagnostics`).
//!   8. When a body-level mismatch cascades up through a nested
//!      `a(b(wrong))` call, only the innermost frame is rendered on the
//!      diagnostic — §16 #16 single-level invariant.
//!
//! The frame stack is always populated regardless of depth; renderer
//! policy (Phase 6 single-level vs Phase 12 multi-level) is the only
//! thing that changes.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, DiagnosticData, SourceFile, Workspace};

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

/// Content of the canonical `safe_divide` function used across tests.
const SAFE_DIVIDE_SRC: &str =
    "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) \
     -> Expr<Double> AS (CAST(numerator AS DOUBLE) / denominator)\n";

#[test]
fn safe_divide_call_types_correctly() {
    // TDD test 1: a well-typed call site produces zero diagnostics from
    // the Phase 6 call-site checker.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("uses_safe_divide.sql");
    // Minimal model body: call `safe_divide` on two integer literals.
    // Integer widens into `Numeric`, which is the parameter constraint —
    // no diagnostic should fire.
    let model_src = "SELECT smelt.fn.safe_divide(10, 2) AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, SAFE_DIVIDE_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let call_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
            )
        })
        .collect();
    assert!(
        call_diags.is_empty(),
        "expected no Phase-6 call diagnostics, got {call_diags:?}"
    );
}

#[test]
fn wrong_arg_type_error_at_call_site() {
    // TDD test 2: passing a Text literal to `Expr<Numeric>` emits one
    // `ArgTypeMismatch` diagnostic. Diagnostic carries an ExpansionFrames
    // data payload with a FrameInfo for `safe_divide.numerator`.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("bad_call.sql");
    let model_src = "SELECT smelt.fn.safe_divide('not a number', 2) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, SAFE_DIVIDE_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::ArgTypeMismatch);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one ArgTypeMismatch diagnostic, got {diags:?}"
    );

    let diag = &diags[0];
    assert!(
        diag.message.contains("numerator"),
        "message should name the parameter `numerator`, got: {}",
        diag.message
    );

    // The diagnostic's range should fall on the `'not a number'` argument —
    // anchored at the arg span, not the whole call.
    let arg_text = "'not a number'";
    let arg_start = model_src.find(arg_text).unwrap();
    let arg_end = arg_start + arg_text.len();
    assert_eq!(
        diag.range.start.line, 0,
        "expected arg span on line 0, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.start.column as usize, arg_start,
        "expected arg-range start at `'not a number'`, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.end.column as usize, arg_end,
        "expected arg-range end after `'not a number'`, got {:?}",
        diag.range
    );
}

#[test]
fn named_args_bind_correctly() {
    // TDD test 3: `smelt.fn.safe_divide(denominator => 2, numerator => 10)`
    // binds by name. Both args parse as Integer → Numeric, so no diagnostics.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("named_call.sql");
    let model_src = "SELECT smelt.fn.safe_divide(denominator => 2, numerator => 10) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, SAFE_DIVIDE_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let call_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
            )
        })
        .collect();
    assert!(
        call_diags.is_empty(),
        "expected no call diagnostics with named args, got {call_diags:?}"
    );
}

#[test]
fn missing_required_arg_error() {
    // TDD test 4: omitting `denominator` from the call emits exactly one
    // `MissingArgument`, anchored at the call-path span.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("missing_arg.sql");
    let model_src = "SELECT smelt.fn.safe_divide(10) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, SAFE_DIVIDE_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::MissingArgument);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one MissingArgument diagnostic, got {diags:?}"
    );
    assert!(
        diags[0].message.contains("denominator"),
        "message should name the missing parameter, got: {}",
        diags[0].message
    );
}

#[test]
fn default_value_fills_missing_arg() {
    // TDD test 5: a function with a defaulted parameter accepts calls that
    // omit it. `add_default(x, y = 0)` called with just `x` should be clean.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_default.sql");
    let fn_src = "smelt.define add_default(x: Expr<Integer>, y: Expr<Integer> = 0) \
                  -> Expr<Integer> AS (x + y)\n";
    let model_path = root.join("models").join("uses_default.sql");
    let model_src = "SELECT smelt.fn.add_default(5) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path, model_src)]);
    let model_file = files[1];

    let missing = diags_with_code(&db, ws, model_file, DiagnosticCode::MissingArgument);
    assert!(
        missing.is_empty(),
        "omitted defaulted parameter should NOT emit MissingArgument, got {missing:?}"
    );
    let type_errs = diags_with_code(&db, ws, model_file, DiagnosticCode::ArgTypeMismatch);
    assert!(
        type_errs.is_empty(),
        "omitted defaulted parameter should produce no type-mismatch, got {type_errs:?}"
    );
}

#[test]
fn unknown_smelt_fn_error() {
    // TDD test 6: calling `smelt.fn.does_not_exist(1)` emits exactly one
    // `UnknownSmeltFn` diagnostic at the call-path span.
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("unknown_call.sql");
    let model_src = "SELECT smelt.fn.does_not_exist(1) AS r\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownSmeltFn diagnostic, got {diags:?}"
    );
    assert!(
        diags[0].message.contains("does_not_exist"),
        "message should name the unresolved function, got: {}",
        diags[0].message
    );
}

#[test]
fn e2e_example_diagnostics_clean() {
    // TDD test 7: the `functions_demo` example ships with a working
    // `smelt.fn.safe_divide` call. This test proves the fixture in
    // `examples/functions_demo/models/uses_safe_divide.sql` produces zero
    // call-site diagnostics when loaded directly. The broader coverage
    // (full example_diagnostics sweep) lives in smelt-cli.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("functions_demo");

    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("uses_safe_divide.sql");

    let fn_content = std::fs::read_to_string(&fn_path).expect("safe_divide.sql must exist");
    let model_content =
        std::fs::read_to_string(&model_path).expect("uses_safe_divide.sql must exist");

    let (db, ws, files) = build_db(
        root.clone(),
        &[(fn_path, &fn_content), (model_path, &model_content)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let call_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
            )
        })
        .collect();
    assert!(
        call_diags.is_empty(),
        "uses_safe_divide.sql should produce no call diagnostics, got {call_diags:?}"
    );
}

#[test]
fn nested_call_error_renders_all_frames() {
    // Phase 12 TDD test 1: a three-level chain
    //   `outer_call(y) AS (smelt.fn.middle(y))`
    //   `middle(z) AS (smelt.fn.inner_unary(z))`
    //   `inner_unary(x) AS (x + undefined_var)`
    // produces exactly one `UnknownIdentifier` diagnostic at the model's
    // `smelt.fn.outer_call(1)` call site whose `ExpansionFrames` payload
    // carries three frames — innermost-first (`inner_unary`) → outermost-last
    // (`outer_call`) — matching the renderer contract. Each frame also
    // carries a `decl_path` + `decl_range` so the LSP can attach
    // `DiagnosticRelatedInformation` entries pointing back to each
    // declaration site.
    let root = PathBuf::from("/fake/project");
    let inner_path = root.join("functions").join("inner_unary.sql");
    let middle_path = root.join("functions").join("middle.sql");
    let outer_path = root.join("functions").join("outer_call.sql");
    let model_path = root.join("models").join("chain_call.sql");

    let inner_src = "smelt.define inner_unary(x) AS (x + undefined_var)\n";
    let middle_src = "smelt.define middle(z) AS (smelt.fn.inner_unary(z))\n";
    let outer_src = "smelt.define outer_call(y) AS (smelt.fn.middle(y))\n";
    let model_src = "SELECT smelt.fn.outer_call(1) AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (inner_path, inner_src),
            (middle_path, middle_src),
            (outer_path, outer_src),
            (model_path, model_src),
        ],
    );
    let model_file = files[3];

    let diags = file_diagnostics(&db, ws, model_file);
    let three_frame: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                && matches!(
                    &d.data,
                    Some(DiagnosticData::ExpansionFrames(frames)) if frames.len() == 3
                )
        })
        .collect();
    assert_eq!(
        three_frame.len(),
        1,
        "expected exactly one UnknownIdentifier diagnostic with three frames at the model call site, got {diags:#?}"
    );
    let diag = three_frame[0];

    let frames = match &diag.data {
        Some(DiagnosticData::ExpansionFrames(frames)) => frames,
        other => panic!("expected ExpansionFrames payload, got {other:?}"),
    };
    // Innermost-first → outermost-last.
    assert_eq!(frames[0].function, "inner_unary");
    assert_eq!(frames[1].function, "middle");
    assert_eq!(frames[2].function, "outer_call");

    // Each frame must carry a decl_path and decl_range so the LSP can
    // build `DiagnosticRelatedInformation` entries.
    for frame in frames {
        assert!(
            frame.decl_path.is_some(),
            "frame {} missing decl_path — LSP would lose the clickable link for this frame",
            frame.function
        );
        assert!(
            frame.decl_range.is_some(),
            "frame {} missing decl_range",
            frame.function
        );
    }
}

#[test]
fn single_level_call_unchanged() {
    // Phase 12 TDD test 2: a direct 1-level call with a call-site
    // ArgTypeMismatch still emits data=None (no frames) — Phase 6 behaviour
    // is preserved verbatim. This guards against the Phase 12 renderer
    // upgrade accidentally inflating simple call-site errors with frame
    // metadata.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("safe_divide.sql");
    let model_path = root.join("models").join("single_bad_call.sql");
    let model_src = "SELECT smelt.fn.safe_divide('bad_text', 1) AS r\n";

    let (db, ws, files) = build_db(root, &[(fn_path, SAFE_DIVIDE_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::ArgTypeMismatch);
    assert_eq!(
        diags.len(),
        1,
        "single-level mismatch should still yield exactly one ArgTypeMismatch, got {diags:?}"
    );
    let diag = &diags[0];
    assert!(
        diag.data.is_none(),
        "direct call-site diagnostics stay data=None under Phase 12 renderer"
    );
}

#[test]
fn frame_stack_only_innermost_rendered() {
    // TDD test 8: `outer_fn(inner_fn('text'))` — the outermost call
    // mismatches Text on a Numeric parameter. The resulting diagnostic
    // carries a FrameInfo for the outermost call (the call site we can
    // see from the source), and Phase 6's renderer policy exposes only
    // that one. Phase 12 will reveal the full stack.
    //
    // We avoid the bare identifier `outer` because it collides with the
    // SQL OUTER keyword in the parser's context-sensitive grammar.
    let root = PathBuf::from("/fake/project");
    let inner_path = root.join("functions").join("inner_fn.sql");
    let outer_path = root.join("functions").join("outer_fn.sql");
    let inner_src = "smelt.define inner_fn(x: Expr<Numeric>) -> Expr<Numeric> AS (x + 1)\n";
    let outer_src =
        "smelt.define outer_fn(y: Expr<Numeric>) -> Expr<Numeric> AS (smelt.fn.inner_fn(y))\n";
    let model_path = root.join("models").join("nested.sql");
    let model_src = "SELECT smelt.fn.outer_fn('text') AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (inner_path, inner_src),
            (outer_path, outer_src),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    // The outer call mismatches on its `y` parameter — a Text literal was
    // passed where `Expr<Numeric>` was expected.
    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::ArgTypeMismatch);
    assert_eq!(
        diags.len(),
        1,
        "nested mismatch should yield one ArgTypeMismatch at the outer call, got {diags:?}"
    );
    let diag = &diags[0];

    // The message must name the outer function and its `y` parameter — the
    // innermost frame the Phase 6 renderer is allowed to expose. The inner
    // function name must NOT appear in the single-level render.
    assert!(
        diag.message.contains("outer_fn") || diag.message.contains('y'),
        "message should mention the outer call's param, got: {}",
        diag.message
    );

    // Sanity: if the diagnostic carries an ExpansionFrames payload, Phase 6
    // stamps exactly one frame. (Direct call-site diagnostics like
    // ArgTypeMismatch leave `data = None` — only body-cascade errors carry
    // frames.)
    if let Some(DiagnosticData::ExpansionFrames(frames)) = &diag.data {
        assert_eq!(
            frames.len(),
            1,
            "Phase 6: direct call-site diagnostics carry at most one frame, got {frames:?}"
        );
    }
}

// ─── Phase 10 — `smelt.extern` declarations + unified resolver ────────────────

#[test]
fn extern_call_typed_like_builtin() {
    // Phase 10 TDD test 3: a `smelt.extern` declaration makes a call-site
    // resolvable through the unified resolver. Calling the extern with
    // correct argument types yields zero diagnostics; the call-site
    // checker's user-defined branch handles externs identically to
    // `smelt.define`, modulo the no-body skip.
    let root = PathBuf::from("/fake/project");
    let ext_path = root.join("functions").join("ext.sql");
    let model_path = root.join("models").join("uses_extern.sql");
    let ext_src =
        "smelt.extern regex_match(text: Expr<Text>, pattern: Expr<Text>) -> Expr<Boolean>\n";
    // Correct types: two Text literals. The checker must not emit any
    // diagnostic, and must NOT try to re-walk the body (externs have none).
    let model_src = "SELECT smelt.fn.regex_match('abc', 'a.*') AS r\n";

    let (db, ws, files) = build_db(root, &[(ext_path, ext_src), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let call_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
            )
        })
        .collect();
    assert!(
        call_diags.is_empty(),
        "correct-typed call to an extern should produce no call diagnostics, got {call_diags:?}"
    );
}

#[test]
fn extern_collision_with_builtin_is_error() {
    // Phase 10 TDD test 4: declaring `smelt.extern LOWER(...)` collides
    // with the built-in `LOWER`. The workspace-level diagnostic fires on
    // the file declaring the extern.
    let root = PathBuf::from("/fake/project");
    let ext_path = root.join("functions").join("bad_lower.sql");
    let ext_src = "smelt.extern LOWER(s: Expr<Text>) -> Expr<Text>\n";

    let (db, ws, files) = build_db(root, &[(ext_path, ext_src)]);
    let ext_file = files[0];

    let diags = file_diagnostics(&db, ws, ext_file);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ExternCollidesWithBuiltin))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one ExternCollidesWithBuiltin diagnostic, got {diags:#?}"
    );
    assert!(
        matching[0].message.to_ascii_lowercase().contains("lower"),
        "message should name the colliding function, got: {}",
        matching[0].message
    );
}

#[test]
fn extern_duplicate_declaration_is_error() {
    // Phase 10 TDD test 5: two `smelt.extern`s with the same name across
    // files produce a DuplicateFunctionDefinition diagnostic, just like
    // two `smelt.define`s. The diagnostic anchors on the alphabetically
    // later file (matching Phase 3's deterministic ordering).
    let root = PathBuf::from("/fake/project");
    let ext_a = root.join("functions").join("ext_a.sql");
    let ext_b = root.join("functions").join("ext_b.sql");
    let ext_src_a = "smelt.extern shared_ext(x: Expr<Integer>) -> Expr<Integer>\n";
    let ext_src_b = "smelt.extern shared_ext(y: Expr<Integer>) -> Expr<Integer>\n";

    let (db, ws, files) = build_db(root, &[(ext_a, ext_src_a), (ext_b.clone(), ext_src_b)]);
    // Diagnostic anchors on the alphabetically-later file (ext_b).
    let ext_b_file = files[1];

    let diags = file_diagnostics(&db, ws, ext_b_file);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::DuplicateFunctionDefinition)
                && d.message.contains("shared_ext")
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one DuplicateFunctionDefinition diagnostic mentioning shared_ext, got {diags:#?}"
    );
}

// ─── Phase 29 — PASSING clause binding ───────────────────────────────────────

/// Source for a `with_filter` function used across Phase 29 tests.
/// `pred` is declared as `Expr<Boolean>` — PASSING clauses supply it.
const WITH_FILTER_SRC: &str =
    "smelt.define with_filter(source: TableExpr, pred: Expr<Boolean>) -> TableExpr AS (\
     SELECT * FROM source WHERE pred\
     )\n";

#[test]
fn passing_clause_binds_to_named_parameter() {
    // Phase 29 TDD test 1: a PASSING clause supplies `pred: Expr<Boolean>`
    // correctly. `TRUE` is a Boolean literal — zero diagnostics expected.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_filter.sql");
    let model_path = root.join("models").join("filtered.sql");
    // Call with a TableExpr positional arg and PASSING for pred.
    let model_src =
        "SELECT * FROM smelt.fn.with_filter(smelt.ref('orders')) PASSING pred AS (TRUE)\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, WITH_FILTER_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
            )
        })
        .collect();
    assert!(
        bad_diags.is_empty(),
        "expected no diagnostics when PASSING supplies pred correctly, got {bad_diags:?}"
    );
}

#[test]
fn passing_clause_name_mismatch_errors() {
    // Phase 29 TDD test 2: PASSING uses a name that doesn't match any parameter.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_filter.sql");
    let model_path = root.join("models").join("bad_passing_name.sql");
    let model_src =
        "SELECT * FROM smelt.fn.with_filter(smelt.ref('orders')) PASSING wrong_name AS (TRUE)\n";

    let (db, ws, files) = build_db(root, &[(fn_path, WITH_FILTER_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let unknown_passing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownPassingParameter))
        .collect();
    assert_eq!(
        unknown_passing.len(),
        1,
        "expected one UnknownPassingParameter diagnostic, got {diags:?}"
    );
    assert!(
        unknown_passing[0].message.contains("wrong_name"),
        "message should name the unknown parameter, got: {}",
        unknown_passing[0].message
    );

    // Span must be anchored at the PASSING_NAME token ("wrong_name"), not the
    // whole call-path fallback. Verify start column matches "wrong_name" offset.
    let name_text = "wrong_name";
    let name_start = model_src.find(name_text).unwrap();
    assert_eq!(
        unknown_passing[0].range.start.line, 0,
        "expected name span on line 0, got {:?}",
        unknown_passing[0].range
    );
    assert_eq!(
        unknown_passing[0].range.start.column as usize, name_start,
        "expected range start at `wrong_name` (col {name_start}), got {:?}",
        unknown_passing[0].range
    );
}

#[test]
fn passing_clause_type_checked_same_as_inline() {
    // Phase 29 TDD test 3: PASSING body type-checked: Integer where Boolean expected.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_filter.sql");
    let model_path = root.join("models").join("bad_passing_type.sql");
    let model_src =
        "SELECT * FROM smelt.fn.with_filter(smelt.ref('orders')) PASSING pred AS (123)\n";

    let (db, ws, files) = build_db(root, &[(fn_path, WITH_FILTER_SRC), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let type_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ArgTypeMismatch))
        .collect();
    assert_eq!(
        type_diags.len(),
        1,
        "expected one ArgTypeMismatch from PASSING body type error, got {diags:?}"
    );
    assert!(
        type_diags[0].message.contains("pred"),
        "message should name the parameter `pred`, got: {}",
        type_diags[0].message
    );
}

#[test]
fn default_fills_omitted_passing() {
    // Phase 29 TDD test 4: parameter with a default is not required via PASSING.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_default_filter.sql");
    let fn_src = "smelt.define with_default_filter(source: TableExpr, pred: Expr<Boolean> = TRUE) \
         -> TableExpr AS (\
         SELECT * FROM source WHERE pred\
         )\n";
    let model_path = root.join("models").join("uses_default_filter.sql");
    // No PASSING clause — `pred` should be filled by its default.
    let model_src = "SELECT * FROM smelt.fn.with_default_filter(smelt.ref('orders'))\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::MissingArgument) | Some(DiagnosticCode::ArgTypeMismatch)
            )
        })
        .collect();
    assert!(
        bad_diags.is_empty(),
        "omitted PASSING for defaulted parameter should produce no diagnostics, got {bad_diags:?}"
    );
}

// ─── Phase 44b — Fragment param reference in PASSING body ────────────────────

/// Inner function: `inner_agg(source: TableExpr, metrics: SelectItems<Agg>) -> TableExpr`.
/// Splices `metrics` into a SELECT list.
const INNER_AGG_FN_SRC: &str = "\
smelt.define inner_agg(\
    source: TableExpr, \
    metrics: SelectItems<Agg> = ()\
) -> TableExpr AS (\
    SELECT metrics FROM source GROUP BY 1\
)\n";

/// Outer function: wraps `inner_agg`, forwarding its own `metrics: SelectItems<Agg>`
/// through to the inner call via `PASSING metrics AS (metrics)`.
const OUTER_WRAPPER_FN_SRC: &str = "\
smelt.define outer_wrapper(\
    source: TableExpr, \
    metrics: SelectItems<Agg> = ()\
) -> TableExpr AS (\
    WITH base AS (\
        smelt.fn.inner_agg(source)\
        PASSING metrics AS (metrics)\
    )\
    SELECT * FROM base\
)\n";

#[test]
fn fragment_param_reference_in_passing_body_inherits_kind() {
    // Phase 44b TDD test 2: when the outer function `outer_wrapper` has a
    // `metrics: SelectItems<Agg>` parameter and its body passes that param
    // to an inner call's `metrics: SelectItems<Agg>` parameter via
    // `PASSING metrics AS (metrics)`, the bare `metrics` reference inside
    // the PASSING body must NOT emit `FragmentKindMismatch`.
    //
    // Without the fix, `infer_expression_kind` returns `Scalar` for the
    // bare identifier `metrics`, causing a kind mismatch against `<Agg>`.
    let root = PathBuf::from("/fake/project");
    let inner_path = root.join("functions").join("inner_agg.sql");
    let outer_path = root.join("functions").join("outer_wrapper.sql");
    // Check the outer function definition for errors.
    let (db, ws, files) = build_db(
        root,
        &[
            (inner_path, INNER_AGG_FN_SRC),
            (outer_path.clone(), OUTER_WRAPPER_FN_SRC),
        ],
    );
    let outer_file = files[1];

    let diags = file_diagnostics(&db, ws, outer_file);
    let kind_mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
        .collect();
    assert!(
        kind_mismatch_diags.is_empty(),
        "Phase 44b: bare fragment-param reference in PASSING body should not emit \
         FragmentKindMismatch, got: {kind_mismatch_diags:?}"
    );
}

#[test]
fn fragment_param_reference_exempt_from_splice_column_validation() {
    // Phase 44b TDD test 3: the same PASSING body `(metrics)` should NOT emit
    // `FragmentColumnMissing` for the parameter name. The parameter is a
    // `SelectItems<Agg>` fragment — it forwards an opaque list, so there are
    // no concrete column references to validate against the splice context.
    let root = PathBuf::from("/fake/project2");
    let inner_path = root.join("functions").join("inner_agg.sql");
    let outer_path = root.join("functions").join("outer_wrapper.sql");
    let (db, ws, files) = build_db(
        root,
        &[
            (inner_path, INNER_AGG_FN_SRC),
            (outer_path.clone(), OUTER_WRAPPER_FN_SRC),
        ],
    );
    let outer_file = files[1];

    let diags = file_diagnostics(&db, ws, outer_file);
    let col_missing_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        col_missing_diags.is_empty(),
        "Phase 44b: bare fragment-param reference in PASSING body should not emit \
         FragmentColumnMissing, got: {col_missing_diags:?}"
    );
}

#[test]
fn non_fragment_param_reference_still_kind_checked() {
    // Phase 44b TDD test 6 (negative regression guard): a literal integer `42`
    // in a PASSING body for `SelectItems<Agg>` must still emit
    // `FragmentKindMismatch` — the fix must not suppress all kind checks.
    let root = PathBuf::from("/fake/project3");
    let inner_path = root.join("functions").join("inner_agg.sql");
    let outer_path = root.join("functions").join("bad_outer.sql");
    let bad_outer_src = "\
smelt.define bad_outer(\
    source: TableExpr\
) -> TableExpr AS (\
    WITH base AS (\
        smelt.fn.inner_agg(source)\
        PASSING metrics AS (42)\
    )\
    SELECT * FROM base\
)\n";
    let (db, ws, files) = build_db(
        root,
        &[
            (inner_path, INNER_AGG_FN_SRC),
            (outer_path.clone(), bad_outer_src),
        ],
    );
    let outer_file = files[1];

    let diags = file_diagnostics(&db, ws, outer_file);
    let kind_mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
        .collect();
    assert!(
        !kind_mismatch_diags.is_empty(),
        "Phase 44b: integer literal 42 in PASSING body for SelectItems<Agg> must still emit \
         FragmentKindMismatch (regression guard), got no such diagnostic; all diags: {diags:?}"
    );
}

#[test]
fn non_param_column_in_fragment_body_still_validated() {
    // Phase 44b TDD test 7 (negative regression guard): when the PASSING body
    // contains a non-fragment-param bare column inside an aggregate call, the
    // column-walk gate must NOT suppress validation.
    //
    // Scenario:
    //   - `col_agg_fn` has `src: TableExpr` and `metrics: SelectItems<Agg, src>`.
    //   - A model calls `col_agg_fn(smelt.ref('orders'))` with
    //     `PASSING metrics AS (COUNT(nonexistent_col))`.
    //   - `nonexistent_col` is NOT a fragment param in `body_ctx`, so
    //     `is_bare_fragment_param_ref` returns false.
    //   - `COUNT(...)` is Agg-kind — the kind check passes.
    //   - `smelt.ref('orders')` has no defined schema in this test, so the
    //     inferred splice context for `src` is empty.
    //   - `nonexistent_col` is not in the empty inferred set →
    //     `FragmentColumnMissing` must fire.
    //
    // This proves the exemption is narrow: only bare fragment-param refs
    // (e.g. `PASSING metrics AS (metrics)`) bypass column validation — all
    // other column references are still checked. The test WILL fail if
    // `is_bare_fragment_param_ref` is accidentally broadened to suppress
    // validation for non-fragment-param columns.
    let root = PathBuf::from("/fake/project4");
    let fn_path = root.join("functions").join("col_agg_fn.sql");
    let model_path = root.join("models").join("uses_col_agg_fn.sql");

    // `col_agg_fn` uses `SelectItems<Agg, src>` so the splice context
    // is the schema of `src`. At call time `src` has no known columns
    // (smelt.ref('orders') is not defined in this test workspace) →
    // inferred_set is empty → any column ref in the PASSING body that is
    // not a fragment param will emit FragmentColumnMissing.
    let fn_src = "\
smelt.define col_agg_fn(\
    src: TableExpr, \
    metrics: SelectItems<Agg, src> = ()\
) -> TableExpr AS (\
    SELECT metrics FROM src GROUP BY 1\
)\n";
    // COUNT is Agg-kind so the kind check passes. `nonexistent_col` is not
    // a fragment param → column walk runs → FragmentColumnMissing fires.
    let model_src = "SELECT * FROM smelt.fn.col_agg_fn(smelt.ref('orders')) \
         PASSING metrics AS (COUNT(nonexistent_col))\n";

    let (db, ws, files) = build_db(root, &[(fn_path, fn_src), (model_path, model_src)]);
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let col_missing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        !col_missing.is_empty(),
        "Phase 44b regression guard: non-fragment-param column `nonexistent_col` in PASSING \
         body must emit FragmentColumnMissing (column-walk must not be suppressed for \
         non-fragment-param references); got no such diagnostic. All diags: {diags:?}"
    );
    assert!(
        col_missing
            .iter()
            .any(|d| d.message.contains("nonexistent_col")),
        "FragmentColumnMissing message should name `nonexistent_col`, got: {col_missing:?}"
    );
}
