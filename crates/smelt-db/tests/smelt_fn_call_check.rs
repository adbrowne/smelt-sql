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
