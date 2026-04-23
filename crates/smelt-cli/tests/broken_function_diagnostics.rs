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

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

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

        let diags = file_diagnostics(&db, ws, fixture_handle);
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
