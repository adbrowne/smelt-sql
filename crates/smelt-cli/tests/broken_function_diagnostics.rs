//! Unified broken-fixture harness for the smelt-functions work (Phases 3+).
//!
//! Every `examples/broken/models/fn_*.sql` fixture asserts its expected
//! diagnostic code + message substring here. This is the single place that
//! later phases (Phase 7+) extend — new broken fixtures add a row to
//! `CASES`, not a whole new test file.
//!
//! Design notes:
//!   - Fixtures that require a second file (e.g. the duplicate-define
//!     pair) carry a `companion` path. The DB is seeded with both files
//!     so the diagnostic is visible on the fixture under test.
//!   - Fixtures that live standalone carry `companion: None`.
//!   - The harness asserts *at least one* matching diagnostic (code +
//!     message substring). It does NOT assert exclusivity — Phase 6's
//!     per-phase tests do the strict counts.

use std::path::PathBuf;

use smelt_db::{
    check_type_diagnostics, file_diagnostics, Database, DiagnosticAcc, DiagnosticCode, SourceFile,
    Workspace,
};

/// A broken-fixture case: which file to load, its expected diagnostic
/// code, and a message substring the diagnostic must contain.
struct Case {
    /// File under test, relative to `examples/broken/models/`.
    fixture: &'static str,
    /// Optional second file loaded into the same workspace (also relative
    /// to `examples/broken/models/`). Needed for fixtures whose diagnostic
    /// only fires in the presence of another declaration (e.g. duplicate
    /// function names across files).
    companion: Option<&'static str>,
    /// Diagnostic code expected on the fixture.
    code: DiagnosticCode,
    /// Substring the diagnostic's message must contain.
    message_substring: &'static str,
}

const CASES: &[Case] = &[
    // Phase 3 — duplicate-define across files. The diagnostic anchors on
    // `fn_duplicate_define_other.sql` (the alphabetically-later file).
    Case {
        fixture: "fn_duplicate_define_other.sql",
        companion: Some("fn_duplicate_define.sql"),
        code: DiagnosticCode::DuplicateFunctionDefinition,
        message_substring: "shared_name",
    },
    // Phase 4 — unsupported sort (`TableExpr<T>`) in a parameter annotation.
    Case {
        fixture: "fn_bad_type_ref.sql",
        companion: None,
        code: DiagnosticCode::InvalidFunctionTypeRef,
        message_substring: "TableExpr",
    },
    // Phase 5 — body Integer + Text type mismatch.
    Case {
        fixture: "fn_body_type_mismatch.sql",
        companion: None,
        code: DiagnosticCode::FunctionBodyTypeMismatch,
        message_substring: "`+`",
    },
    // Phase 5 — bare identifier in a body with no matching parameter.
    Case {
        fixture: "fn_unknown_param.sql",
        companion: None,
        code: DiagnosticCode::UnknownIdentifier,
        message_substring: "z",
    },
    // Phase 6 — call-site argument type mismatch (Text passed to Numeric).
    Case {
        fixture: "fn_call_wrong_arg_type.sql",
        companion: None,
        code: DiagnosticCode::ArgTypeMismatch,
        message_substring: "needs_number",
    },
    // Phase 6 — call-site required argument omitted.
    Case {
        fixture: "fn_call_missing_arg.sql",
        companion: None,
        code: DiagnosticCode::MissingArgument,
        message_substring: "takes_two",
    },
    // Phase 6 — call-site references a function that isn't declared
    // anywhere in the workspace.
    Case {
        fixture: "fn_call_unknown.sql",
        companion: None,
        code: DiagnosticCode::UnknownSmeltFn,
        message_substring: "does_not_exist",
    },
    // Phase 8 (landed in Phase 10) — `COALESCE(text, int)` violates the
    // shared-type-variable constraint on the variadic generic built-in.
    Case {
        fixture: "fn_coalesce_text_int.sql",
        companion: None,
        code: DiagnosticCode::ArgTypeMismatch,
        message_substring: "COALESCE",
    },
    // Phase 8 (landed in Phase 10) — `GREATEST()` has no args; registry
    // arity check yields MissingArgument.
    Case {
        fixture: "fn_greatest_no_args.sql",
        companion: None,
        code: DiagnosticCode::MissingArgument,
        message_substring: "GREATEST",
    },
    // Phase 10 — `smelt.extern LOWER(...)` collides with the canonical
    // built-in of the same name.
    Case {
        fixture: "fn_extern_collides_with_builtin.sql",
        companion: None,
        code: DiagnosticCode::ExternCollidesWithBuiltin,
        message_substring: "LOWER",
    },
    // Phase 10 — two `smelt.extern`s with the same name across sibling
    // files. Diagnostic anchors on the alphabetically-later file.
    Case {
        fixture: "fn_extern_duplicate_other.sql",
        companion: Some("fn_extern_duplicate.sql"),
        code: DiagnosticCode::DuplicateFunctionDefinition,
        message_substring: "extern_twice",
    },
    // Phase 11 — frontmatter declares `[duckdb, spark]` while the body
    // uses `duckdb.read_parquet` (inferred [duckdb]). The narrow-only
    // rule rejects the widening.
    Case {
        fixture: "fn_backends_widening.sql",
        companion: None,
        code: DiagnosticCode::BackendsWideningNotAllowed,
        message_substring: "load_broken",
    },
    // Phase 12 — `outer_call(1)` cascades through three untyped helpers;
    // `inner_unary`'s body references an `undefined_var` identifier,
    // triggering an `UnknownIdentifier` cascade. That diagnostic is
    // wrapped by frames for `middle` and `outer_call` on the way back,
    // yielding three expansion frames. Full outer-to-inner frame
    // ordering is asserted in the dedicated
    // `nested_call_error_renders_all_frames` test below.
    Case {
        fixture: "fn_nested_call_error.sql",
        companion: None,
        code: DiagnosticCode::UnknownIdentifier,
        message_substring: "undefined_var",
    },
    // Phase 14 (§16 #24) — `WHERE ROW_NUMBER() OVER (...) > 1` is a
    // window-kind expression in a scalar splice point. The diagnostic
    // anchors at the offending expression and names ROW_NUMBER.
    Case {
        fixture: "fn_window_in_where.sql",
        companion: None,
        code: DiagnosticCode::WindowInScalarContext,
        message_substring: "ROW_NUMBER",
    },
    // Phase 15 — a `TableExpr`-taking function is called with a caller
    // whose schema lacks the column the body references. The body
    // check runs at call-site expansion; the bare `revenue` reference
    // surfaces as `UnknownIdentifier` rooted at the call site.
    Case {
        fixture: "fn_tableexpr_missing_col.sql",
        companion: Some("fn_tableexpr_missing_col_other.sql"),
        code: DiagnosticCode::UnknownIdentifier,
        message_substring: "revenue",
    },
    // Phase 15 — an `Expr<Text>` parameter shadows a column of the
    // same name in a sibling `TableExpr` parameter's caller schema.
    // Warning severity, anchored at the parameter declaration.
    Case {
        fixture: "fn_tableexpr_shadow_warning.sql",
        companion: Some("fn_tableexpr_shadow_warning_other.sql"),
        code: DiagnosticCode::ParameterShadowsColumn,
        message_substring: "user_id",
    },
    // Phase 16 — call a `TableExpr<{revenue: Numeric, cost: Numeric}>`
    // function against a caller whose schema lacks `cost`. The row-
    // requirement check fires at the call site before body expansion.
    Case {
        fixture: "fn_row_requirement_missing.sql",
        companion: Some("fn_row_requirement_missing_other.sql"),
        code: DiagnosticCode::RowRequirementUnsatisfied,
        message_substring: "cost",
    },
    // Phase 17 — the caller projects `missing_col` through a
    // `TableExpr`-returning call's inferred return schema, but
    // `missing_col` isn't in that schema. Surfaces as
    // `UndeclaredColumn` on the explicit projection.
    Case {
        fixture: "fn_tableexpr_return_bare_col_missing.sql",
        companion: Some("fn_tableexpr_return_bare_col_missing_other.sql"),
        code: DiagnosticCode::UndeclaredColumn,
        message_substring: "missing_col",
    },
    // Phase 20 — CTE `a` and `b` mutually reference each other, forming a
    // cycle that is detected by the two-pass body analyser.
    Case {
        fixture: "fn_cte_cycle.sql",
        companion: None,
        code: DiagnosticCode::CteCycle,
        message_substring: "cyclic",
    },
    // Phase 21: fragment references a column not in the inferred splice context.
    Case {
        fixture: "fn_fragment_col_missing.sql",
        companion: Some("fn_fragment_col_missing_other.sql"),
        code: DiagnosticCode::FragmentColumnMissing,
        message_substring: "nonexistent",
    },
    // Phase 21: annotation claims columns beyond the inferred HAVING context.
    Case {
        fixture: "fn_annotation_too_wide.sql",
        companion: Some("fn_annotation_too_wide_other.sql"),
        code: DiagnosticCode::AnnotationTooWide,
        message_substring: "wider",
    },
    // Phase 23: Tier 2 body type error at definition time. `revenue` is
    // Expr<Integer> but `'text'` is Text — fires FunctionBodyTypeMismatch
    // without any call site.
    Case {
        fixture: "fn_tier2_body_broken.sql",
        companion: None,
        code: DiagnosticCode::FunctionBodyTypeMismatch,
        message_substring: "TEXT",
    },
    // Phase 24: Tier 3 return type mismatch. Body returns Text but
    // declaration says Expr<Integer>.
    Case {
        fixture: "fn_tier3_return_mismatch.sql",
        companion: None,
        code: DiagnosticCode::ReturnTypeMismatch,
        message_substring: "TEXT",
    },
    // Phase 25: Tier 2 call with wrong argument type. `mul_typed_local` expects
    // Expr<Integer> but the literal 'hello' is Text. Expected: ArgTypeMismatch.
    Case {
        fixture: "fn_tier2_call_arg_wrong.sql",
        companion: None,
        code: DiagnosticCode::ArgTypeMismatch,
        message_substring: "mul_typed_local",
    },
    // Phase 26: Tier 2 body calls a Tier 1 helper with a broken body. The Tier 1
    // body `x + 'text'` fails under Integer arg from the Tier 2 caller.
    // Expected: FunctionBodyTypeMismatch cascaded to the Tier 2 definition site.
    Case {
        fixture: "fn_tier2_calls_broken_tier1.sql",
        companion: None,
        code: DiagnosticCode::FunctionBodyTypeMismatch,
        message_substring: "`+`",
    },
    // Phase 31: `provenance:` in frontmatter without `unstable_schema: true` in
    // smelt.yml. The broken workspace has no unstable_schema flag, so this
    // declaration should trigger UnstableSchemaRequired.
    Case {
        fixture: "fn_provenance_no_unstable_flag.sql",
        companion: None,
        code: DiagnosticCode::UnstableSchemaRequired,
        message_substring: "fn_provenance_no_flag",
    },
    // Phase 38: `smelt.as_struct()` with a backend that has no struct-literal
    // support. The declared `backends: [no_struct_db]` includes a backend
    // unknown to `backend_supports_struct_literal`, so the checker fires
    // `AsStructUnsupportedBackend`.
    Case {
        fixture: "fn_as_struct_no_backend_literal.sql",
        companion: None,
        code: DiagnosticCode::AsStructUnsupportedBackend,
        message_substring: "no_struct_db",
    },
    // Phase 41: transparent-function call-graph cycle.  `cycle_a` calls
    // `cycle_b`, which calls back into `cycle_a` — both functions are on
    // the cycle and the diagnostic anchors at each declaration's name.
    Case {
        fixture: "fn_call_cycle_a.sql",
        companion: Some("fn_call_cycle_b.sql"),
        code: DiagnosticCode::FunctionCallCycle,
        message_substring: "cycle_a",
    },
    Case {
        fixture: "fn_call_cycle_b.sql",
        companion: Some("fn_call_cycle_a.sql"),
        code: DiagnosticCode::FunctionCallCycle,
        message_substring: "cycle_b",
    },
    // Phase 43 — frontmatter YAML that fails to parse. Body is valid; the
    // only diagnostic is FrontmatterParseError (severity Error) anchored at
    // the declaration's name range.
    Case {
        fixture: "fn_frontmatter_malformed.sql",
        companion: None,
        code: DiagnosticCode::FrontmatterParseError,
        message_substring: "frontmatter",
    },
    // Phase 43 — frontmatter contains an unknown top-level key. Severity is
    // Warning, not Error; the rest of the frontmatter still parses.
    Case {
        fixture: "fn_frontmatter_unknown_key.sql",
        companion: None,
        code: DiagnosticCode::FrontmatterParseError,
        message_substring: "unknown_property",
    },
    // Phase 45 — body references a column on a JOIN-aliased schema
    // (`y.does_not_exist`) that the joined model does not provide.
    // The body checker now sees the joined alias and surfaces the
    // unknown column as `UnknownIdentifier`.
    Case {
        fixture: "fn_join_alias_missing_col.sql",
        companion: Some("fn_join_alias_missing_col_other.sql"),
        code: DiagnosticCode::UnknownIdentifier,
        message_substring: "does_not_exist",
    },
    // Phase 45 — `Expr<Text>` parameter named `name` collides with a
    // column of the same name on a joined alias's schema. The shadow-
    // warning check now consults JOIN-aliased schemas.
    Case {
        fixture: "fn_join_alias_shadow.sql",
        companion: Some("fn_join_alias_shadow_other.sql"),
        code: DiagnosticCode::ParameterShadowsColumn,
        message_substring: "name",
    },
    // Phase 49 — scalar subquery in WHERE contains a window function.
    // The recursive scalar-subquery check must surface `WindowInScalarContext`.
    Case {
        fixture: "fn_window_in_subquery_where.sql",
        companion: None,
        code: DiagnosticCode::WindowInScalarContext,
        message_substring: "WHERE",
    },
    // Phase 49 — scalar subquery in HAVING contains a window function.
    // The HAVING check (new in Phase 49) must surface `WindowInScalarContext`.
    Case {
        fixture: "fn_window_in_subquery_having.sql",
        companion: None,
        code: DiagnosticCode::WindowInScalarContext,
        message_substring: "HAVING",
    },
];

/// Phase 52 broken fixtures — tested without unstable_schema (extern check is
/// unconditional).
const PHASE52_CASES: &[Case] = &[
    // Phase 52 — extern with a SelectItems parameter is rejected.
    Case {
        fixture: "fn_extern_with_selectitems.sql",
        companion: None,
        code: DiagnosticCode::ExternFragmentParamUnsupported,
        message_substring: "fragment-sort",
    },
];

/// Phase 51 broken fixtures — tested with `unstable_schema: true`.
const PHASE51_CASES: &[Case] = &[
    // Phase 51 — provenance declares a source column (dim.extra) that the body
    // does not read. Must emit ProvenanceMismatch.
    Case {
        fixture: "fn_provenance_extra_col.sql",
        companion: None,
        code: DiagnosticCode::ProvenanceMismatch,
        message_substring: "extra",
    },
    // Phase 51 — joins: declares dim_a but body only joins dim_b. Must emit
    // JoinsMismatch for the undeclared join.
    Case {
        fixture: "fn_joins_mismatch.sql",
        companion: None,
        code: DiagnosticCode::JoinsMismatch,
        message_substring: "dim_a",
    },
];

fn broken_models_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("broken")
        .join("models")
}

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, String)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), content.clone(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn build_db_with_yml(
    project_root: PathBuf,
    files: &[(PathBuf, String)],
    smelt_yml_text: &str,
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, smelt_yml_text.to_string());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), content.clone(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

#[test]
fn no_orphan_fn_fixtures() {
    let models_dir = broken_models_dir();

    // All `fn_*.sql` files currently on disk.
    let mut on_disk: Vec<String> = std::fs::read_dir(&models_dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", models_dir.display()))
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with("fn_") && name.ends_with(".sql") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    on_disk.sort();

    // All fixtures CASES covers — primary + companion entries.
    let mut covered: Vec<String> = CASES
        .iter()
        .chain(PHASE51_CASES.iter())
        .chain(PHASE52_CASES.iter())
        .flat_map(|c| std::iter::once(c.fixture).chain(c.companion))
        .map(|s| s.to_string())
        .collect();
    covered.sort();
    covered.dedup();

    let orphans: Vec<&String> = on_disk.iter().filter(|f| !covered.contains(f)).collect();
    assert!(
        orphans.is_empty(),
        "fn_*.sql fixtures not covered by CASES in broken_function_diagnostics.rs: {orphans:?}. \
         Add a Case entry (or a companion reference) for each orphan.",
    );

    // Guard the other direction too: CASES must not reference files that
    // have been deleted from the fixtures directory.
    let missing: Vec<&String> = covered.iter().filter(|f| !on_disk.contains(f)).collect();
    assert!(
        missing.is_empty(),
        "CASES references fixtures that don't exist on disk: {missing:?}",
    );
}

/// Phase 12 TDD test: verify the multi-frame renderer emits all three
/// frame names in outer-to-inner order on the broken nested fixture.
///
/// The fixture chain is `outer_call → middle → inner_unary` and the
/// body-cascade in `inner_unary` produces three expansion frames. The
/// LSP / CLI trailer format prints the outermost frame first (after the
/// primary error message) and the innermost frame last. The rendered
/// message (error + trailers) must therefore contain the frame names in
/// exactly that order: `outer_call < middle < inner_unary`.
#[test]
fn nested_call_error_renders_all_frames() {
    use smelt_db::DiagnosticData;

    let models_dir = broken_models_dir();
    let project_root = models_dir.parent().unwrap().to_path_buf();
    let fixture_path = models_dir.join("fn_nested_call_error.sql");
    let fixture_content =
        std::fs::read_to_string(&fixture_path).expect("fn_nested_call_error.sql must exist");

    let (db, ws, handles) = build_db(project_root, &[(fixture_path.clone(), fixture_content)]);
    let fixture_handle = handles[0];

    let diags = file_diagnostics(&db, ws, fixture_handle);

    // The fixture produces several UnknownIdentifier diagnostics:
    //   - one "raw" diagnostic from file-level `check_function_body`
    //     on `inner_unary`'s body (no expansion frames).
    //   - cascade copies produced by each outer call site's body re-walk,
    //     carrying progressively more frames (1, 2, 3).
    // The 3-frame diagnostic is the one anchored at the outermost
    // (model-level) `smelt.fn.outer_call(1)` call site — that's the
    // rendered user-facing error.
    let cascades: Vec<_> = diags
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
        cascades.len(),
        1,
        "expected exactly one UnknownIdentifier diagnostic carrying three expansion frames, got {diags:#?}"
    );
    let diag = cascades[0];

    // The diagnostic must carry an ExpansionFrames payload with exactly
    // three frames (innermost-first → outermost-last, matching the data
    // convention `check_smelt_fn_call` follows).
    let frames = match &diag.data {
        Some(DiagnosticData::ExpansionFrames(frames)) => frames,
        other => panic!("expected ExpansionFrames on the cascade diagnostic, got {other:?}"),
    };
    assert_eq!(
        frames.len(),
        3,
        "expected three expansion frames, got {frames:#?}"
    );
    assert_eq!(frames[0].function, "inner_unary");
    assert_eq!(frames[1].function, "middle");
    assert_eq!(frames[2].function, "outer_call");

    // Render the outer-to-inner trailer chain the same way the LSP does:
    // iterate frames in reverse and append one line per frame. The
    // resulting message must list `outer_call` before `middle` before
    // `inner_unary` — the §16 #16 Step 2 renderer contract.
    let mut rendered = diag.message.clone();
    for f in frames.iter().rev() {
        rendered.push_str(&format!(
            "\nin expansion of `{}`, `{}` was bound to {}",
            f.function, f.param, f.bound_type,
        ));
    }
    let pos_outer = rendered
        .find("outer_call")
        .expect("rendered message must mention outer_call");
    let pos_middle = rendered
        .find("middle")
        .expect("rendered message must mention middle");
    let pos_inner = rendered
        .find("inner_unary")
        .expect("rendered message must mention inner_unary");
    assert!(
        pos_outer < pos_middle && pos_middle < pos_inner,
        "frames must render outer-to-inner; got outer@{pos_outer}, middle@{pos_middle}, inner@{pos_inner} — rendered:\n{rendered}"
    );

    // Every frame must carry its declaration-site metadata — the LSP uses
    // these to build `DiagnosticRelatedInformation` entries linking back
    // to each `smelt.define`. `decl_path` may legitimately be `None` only
    // when the declaring file is not in the workspace; in this fixture
    // all three are present, so none should be `None`.
    for frame in frames {
        assert!(
            frame.decl_path.is_some(),
            "frame {} missing decl_path; LSP would fall back to inline-only rendering",
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
fn every_broken_fn_fixture_emits_expected_diagnostic() {
    let models_dir = broken_models_dir();
    let project_root = models_dir.parent().unwrap().to_path_buf();

    for case in CASES {
        let fixture_path = models_dir.join(case.fixture);
        let fixture_content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture {} must exist: {e}", case.fixture));

        let mut files = vec![(fixture_path.clone(), fixture_content)];
        if let Some(companion) = case.companion {
            let companion_path = models_dir.join(companion);
            let companion_content = std::fs::read_to_string(&companion_path)
                .unwrap_or_else(|e| panic!("companion {companion} must exist: {e}"));
            files.push((companion_path, companion_content));
        }

        let (db, ws, handles) = build_db(project_root.clone(), &files);
        let fixture_handle = handles[0];

        // Aggregate both the accumulated `check_file_diagnostics` set
        // and the `check_type_diagnostics` set — Phase 17's
        // UndeclaredColumn diagnostics live in the latter.
        let file_diags = file_diagnostics(&db, ws, fixture_handle);
        let type_diags: Vec<_> =
            check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, fixture_handle)
                .into_iter()
                .map(|d| d.0.clone())
                .collect();
        let diags: Vec<_> = file_diags.into_iter().chain(type_diags).collect();
        let matching: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(case.code) && d.message.contains(case.message_substring))
            .collect();

        assert!(
            !matching.is_empty(),
            "fixture {fix} expected a {code:?} diagnostic containing {msg:?}, \
             got {diags:#?}",
            fix = case.fixture,
            code = case.code,
            msg = case.message_substring,
        );
    }
}

#[test]
fn phase51_provenance_broken_cases() {
    let models_dir = broken_models_dir();
    let project_root = models_dir.parent().unwrap().to_path_buf();
    for case in PHASE51_CASES {
        let fixture_path = models_dir.join(case.fixture);
        let fixture_content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture {} must exist: {e}", case.fixture));
        let files = vec![(fixture_path.clone(), fixture_content)];
        let (db, ws, handles) =
            build_db_with_yml(project_root.clone(), &files, "unstable_schema: true\n");
        let fixture_handle = handles[0];
        let file_diags = file_diagnostics(&db, ws, fixture_handle);
        let type_diags: Vec<_> =
            check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, fixture_handle)
                .into_iter()
                .map(|d| d.0.clone())
                .collect();
        let diags: Vec<_> = file_diags.into_iter().chain(type_diags).collect();
        let matching: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(case.code) && d.message.contains(case.message_substring))
            .collect();
        assert!(
            !matching.is_empty(),
            "fixture {fix} expected a {code:?} diagnostic containing {msg:?}, got {diags:#?}",
            fix = case.fixture,
            code = case.code,
            msg = case.message_substring,
        );
    }
}

#[test]
fn phase52_extern_fragment_param_cases() {
    let models_dir = broken_models_dir();
    let project_root = models_dir.parent().unwrap().to_path_buf();
    for case in PHASE52_CASES {
        let fixture_path = models_dir.join(case.fixture);
        let fixture_content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture {} must exist: {e}", case.fixture));
        let mut files = vec![(fixture_path.clone(), fixture_content)];
        if let Some(companion) = case.companion {
            let companion_path = models_dir.join(companion);
            let companion_content = std::fs::read_to_string(&companion_path)
                .unwrap_or_else(|e| panic!("companion {companion} must exist: {e}"));
            files.push((companion_path, companion_content));
        }
        let (db, ws, handles) = build_db(project_root.clone(), &files);
        let fixture_handle = handles[0];
        let file_diags = file_diagnostics(&db, ws, fixture_handle);
        let type_diags: Vec<_> =
            check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, fixture_handle)
                .into_iter()
                .map(|d| d.0.clone())
                .collect();
        let diags: Vec<_> = file_diags.into_iter().chain(type_diags).collect();
        let matching: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(case.code) && d.message.contains(case.message_substring))
            .collect();
        assert!(
            !matching.is_empty(),
            "fixture {fix} expected a {code:?} diagnostic containing {msg:?}, got {diags:#?}",
            fix = case.fixture,
            code = case.code,
            msg = case.message_substring,
        );
    }
}
