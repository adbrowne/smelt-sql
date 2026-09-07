pub(crate) use line_index::LineIndex;
pub(crate) use smelt_cli::{init_db, Config, ModelDiscovery};
pub(crate) use smelt_db::{DiagnosticAcc, Workspace};
pub(crate) use std::path::Path;

pub(crate) const PHASE_A_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::MetaListEmptyTypeUnknown,
    smelt_db::DiagnosticCode::MetaListHeterogeneous,
    smelt_db::DiagnosticCode::MetaSpreadInForbiddenPosition,
    smelt_db::DiagnosticCode::MetaSpreadOnNonList,
    smelt_db::DiagnosticCode::MetaListInScalarPosition,
];

pub(crate) const PHASE_B_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::LambdaInForbiddenPosition,
    smelt_db::DiagnosticCode::LambdaArityMismatch,
    smelt_db::DiagnosticCode::LambdaZeroParameters,
    smelt_db::DiagnosticCode::LambdaDuplicateParameter,
    smelt_db::DiagnosticCode::LambdaResultTypeMismatch,
    smelt_db::DiagnosticCode::HofExpectsLambda,
    smelt_db::DiagnosticCode::HofExpectsReducer,
    smelt_db::DiagnosticCode::HofNameShadowed,
    smelt_db::DiagnosticCode::ReducerNameShadowed,
    smelt_db::DiagnosticCode::PipeRhsNotCall,
    smelt_db::DiagnosticCode::PipeInDataPosition,
    smelt_db::DiagnosticCode::ReducerInputTypeMismatch,
    smelt_db::DiagnosticCode::ReducerEmptyNoIdentity,
    smelt_db::DiagnosticCode::ReducerArityMismatch,
    smelt_db::DiagnosticCode::ReducerArgTypeMismatch,
    smelt_db::DiagnosticCode::ReducerArgNotCompileTime,
    smelt_db::DiagnosticCode::ReducerNamedArgument,
    smelt_db::DiagnosticCode::HofNamedArgument,
    smelt_db::DiagnosticCode::TernaryConditionNotBoolean,
    smelt_db::DiagnosticCode::TernaryBranchTypeMismatch,
    smelt_db::DiagnosticCode::TernaryKeywordShadowed,
    smelt_db::DiagnosticCode::TernaryInDataPosition,
    smelt_db::DiagnosticCode::TernaryDanglingThen,
    smelt_db::DiagnosticCode::TernaryDanglingElse,
    smelt_db::DiagnosticCode::ConfigVarNotFound,
    smelt_db::DiagnosticCode::ConfigVarNameNotLiteral,
    smelt_db::DiagnosticCode::ConfigVarNullCoercion,
];

pub(crate) const PHASE_C_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::ColumnsOfRequiresTableExpr,
    smelt_db::DiagnosticCode::ColumnsOfNamedArgument,
    smelt_db::DiagnosticCode::ColumnsOfUnresolvableSchema,
    smelt_db::DiagnosticCode::ColumnRefFieldUnknown,
];

pub(crate) const PHASE_D_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::WithTagRequiresText,
    smelt_db::DiagnosticCode::WithTagNamedArgument,
    smelt_db::DiagnosticCode::WideReflectionUnknownAccessor,
    smelt_db::DiagnosticCode::WideReflectionUnexpectedArgument,
    smelt_db::DiagnosticCode::ModelRefFieldUnknown,
    smelt_db::DiagnosticCode::SourceRefFieldUnknown,
];

pub(crate) const PHASE_E1_CODES: &[smelt_db::DiagnosticCode] = &[
    // Record codes (declared but not yet wired into file_diagnostics)
    smelt_db::DiagnosticCode::SmeltRecordRedefinition,
    smelt_db::DiagnosticCode::RecordFieldUnknown,
    smelt_db::DiagnosticCode::RecordFieldMissing,
    smelt_db::DiagnosticCode::RecordFieldDuplicate,
    smelt_db::DiagnosticCode::RecordFieldTypeMismatch,
    smelt_db::DiagnosticCode::RecordLiteralUnknownTarget,
    smelt_db::DiagnosticCode::RecordFieldNotProjectable,
    smelt_db::DiagnosticCode::RecordFieldTypeForbidden,
    smelt_db::DiagnosticCode::RecordCyclicDeclaration,
    smelt_db::DiagnosticCode::RecordInDataWorld,
    // Map codes (declared but not yet wired into file_diagnostics)
    smelt_db::DiagnosticCode::MapKeyTypeNotText,
    smelt_db::DiagnosticCode::MapApiUnknown,
    smelt_db::DiagnosticCode::MapApiArityMismatch,
    smelt_db::DiagnosticCode::MapApiNamedArgument,
    smelt_db::DiagnosticCode::MapApiUnexpectedArgument,
    smelt_db::DiagnosticCode::MapGetMissingKey,
    smelt_db::DiagnosticCode::MapApiArgTypeMismatch,
    // Loader codes (wired via loader_call_diagnostics_for_file)
    smelt_db::DiagnosticCode::ConfigLoaderPathNotLiteral,
    smelt_db::DiagnosticCode::ConfigLoaderPathEscapesWorkspace,
    smelt_db::DiagnosticCode::ConfigLoaderPathBackslash,
    smelt_db::DiagnosticCode::ConfigLoaderFileNotFound,
    smelt_db::DiagnosticCode::ConfigLoaderSchemaForbidden,
    smelt_db::DiagnosticCode::ConfigLoaderTomlNotYetSupported,
    smelt_db::DiagnosticCode::ConfigLoaderParseError,
    smelt_db::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
    smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    smelt_db::DiagnosticCode::ConfigLoaderTypeMismatch,
    smelt_db::DiagnosticCode::ConfigLoaderRootShapeMismatch,
    smelt_db::DiagnosticCode::ConfigLoaderDuplicateMapKey,
    smelt_db::DiagnosticCode::ConfigLoaderNullCoercion,
];

pub(crate) const PHASE_E2_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::GeneratesUnknownValue,
    smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    smelt_db::DiagnosticCode::GenerateFileBareSelectForbidden,
    smelt_db::DiagnosticCode::GenerateFileBodyTypeError,
    smelt_db::DiagnosticCode::ModelDefOutsideGeneratorFile,
    smelt_db::DiagnosticCode::ModelDefInvalidName,
    smelt_db::DiagnosticCode::ModelDefInvalidMaterialization,
    smelt_db::DiagnosticCode::ModelDefDuplicateName,
    smelt_db::DiagnosticCode::ModelDefHandAuthoredCollision,
    smelt_db::DiagnosticCode::GeneratorBodyForbidsModelReflection,
];

pub(crate) fn check_workspace_no_diagnostics(example_dir: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();

    // Discover function files under `functions/`. Phase 3 registers them as
    // Salsa `SourceFile` inputs alongside models so the signature index sees
    // them. Workspaces without a `functions/` directory get an empty vec.
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    // Discover Python models (executes Python to get generated SQL)
    let python_files = discovery.discover_python_files().unwrap();
    if !python_files.is_empty() {
        let python_models = smelt_cli::discover_python_models(
            &python_files,
            &models,
            &config,
            &path,
            config.python.as_deref(),
        )
        .unwrap();
        models.extend(python_models);
    }

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_issues = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.message
            ));
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.0.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.0.message
            ));
        }
    }

    assert!(
        all_issues.is_empty(),
        "Found {} diagnostic(s) in {}:\n  {}",
        all_issues.len(),
        example_dir,
        all_issues.join("\n  ")
    );
}

/// Helper: loads `example_dir` as a workspace, then checks diagnostics for the
/// one file whose relative path ends with `expected_file`.  Asserts:
///   1. Exactly one Phase A diagnostic fires for that file (codes in
///      `PHASE_A_CODES`).  ParseError is allowed alongside it because the
///      WHERE-spread model necessarily produces a parser error in addition to
///      `MetaSpreadInForbiddenPosition`.
///   2. That Phase A diagnostic has code `expected_code`.
///   3. No Phase A diagnostics fire for any OTHER file in the workspace.
///
/// This keeps each broken fixture surgical: one intentionally broken model
/// triggers one specific Phase A diagnostic, and no other file in the workspace
/// triggers any Phase A code.  The broken workspace may contain multiple broken
/// models that are each tested individually.
pub(crate) fn check_workspace_emits_exactly_one_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    // Phase A diagnostics from the target file.
    let mut target_phase_a: Vec<smelt_db::Diagnostic> = Vec::new();
    // Phase A diagnostics from all OTHER files (must be empty).
    let mut other_phase_a: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_a = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_A_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_a(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_a.push(d.clone());
            } else {
                other_phase_a.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_a(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_a.push(d.0.clone());
            } else {
                other_phase_a.push((rel.clone(), d.0.clone()));
            }
        }
    }

    // No other file in the workspace may fire Phase A diagnostics.
    assert!(
        other_phase_a.is_empty(),
        "expected zero Phase A diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_a.len(),
        other_phase_a
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // The target file must produce exactly one Phase A diagnostic.
    assert_eq!(
        target_phase_a.len(),
        1,
        "expected exactly 1 Phase A diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_a.len(),
        target_phase_a
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // Assert the diagnostic has the expected code.
    assert_eq!(
        target_phase_a[0].code,
        Some(expected_code),
        "expected Phase A diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_a[0].code,
        target_phase_a[0].message
    );
}

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase B diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase B codes.
pub(crate) fn check_workspace_emits_exactly_one_phase_b_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_b: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_b: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_b = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_B_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_b(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_b.push(d.clone());
            } else {
                other_phase_b.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_b(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_b.push(d.0.clone());
            } else {
                other_phase_b.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_b.is_empty(),
        "expected zero Phase B diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_b.len(),
        other_phase_b
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_b.len(),
        1,
        "expected exactly 1 Phase B diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_b.len(),
        target_phase_b
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_b[0].code,
        Some(expected_code),
        "expected Phase B diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_b[0].code,
        target_phase_b[0].message
    );
}

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase C diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase C codes.
pub(crate) fn check_workspace_emits_exactly_one_phase_c_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_c: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_c: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_c = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_C_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_c(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_c.push(d.clone());
            } else {
                other_phase_c.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_c(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_c.push(d.0.clone());
            } else {
                other_phase_c.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_c.is_empty(),
        "expected zero Phase C diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_c.len(),
        other_phase_c
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_c.len(),
        1,
        "expected exactly 1 Phase C diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_c.len(),
        target_phase_c
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_c[0].code,
        Some(expected_code),
        "expected Phase C diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_c[0].code,
        target_phase_c[0].message
    );
}

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase D diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase D codes.
pub(crate) fn check_workspace_emits_exactly_one_phase_d_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_d: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_d: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_d = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_D_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_d(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_d.push(d.clone());
            } else {
                other_phase_d.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_d(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_d.push(d.0.clone());
            } else {
                other_phase_d.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_d.is_empty(),
        "expected zero Phase D diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_d.len(),
        other_phase_d
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_d.len(),
        1,
        "expected exactly 1 Phase D diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_d.len(),
        target_phase_d
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_d[0].code,
        Some(expected_code),
        "expected Phase D diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_d[0].code,
        target_phase_d[0].message
    );
}

/// Initialise a Salsa `Database` for `project_dir` / `models` AND register every
/// `.yaml` / `.yml` / `.json` file under `project_dir` as a `LoaderFileInput`.
///
/// This is needed so that `loader_call_diagnostics_for_file_with_content` (step 2)
/// can perform content-validation and emit diagnostics such as
/// `ConfigLoaderRequiredFieldMissing`, `ConfigLoaderParseError`, etc.  The plain
/// `init_db` helper only registers SQL model files; without the loader-file
/// registration the content-validation path is skipped and those diagnostics never
/// fire.
///
/// `sources.yml` / `sources.yaml` are excluded (they are project config, not
/// loader targets).
pub(crate) fn init_db_with_loaders(
    project_dir: &Path,
    models: &[smelt_cli::ModelFile],
) -> smelt_db::Database {
    let mut db = smelt_cli::init_db(project_dir, models);

    // Walk the project directory for YAML / JSON files and register each as a
    // loader file input.
    let walker = walkdir::WalkDir::new(project_dir)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories and the `target/` build artefact tree.
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "target"
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "yaml" | "yml" | "json") {
            continue;
        }
        // Exclude sources.yml / sources.yaml (project config, not loader targets).
        let file_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name == "sources.yml" || file_name == "sources.yaml" {
            continue;
        }
        // Exclude smelt.yml (workspace config).
        if file_name == "smelt.yml" || file_name == "smelt.yaml" {
            continue;
        }

        // Compute workspace-relative path (forward slashes).
        let rel = match entry.path().strip_prefix(project_dir) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        db.set_loader_file(
            std::sync::Arc::from(rel.as_str()),
            std::sync::Arc::from(content.as_str()),
            true,
        );
    }

    db
}

/// Variant of `check_workspace_no_diagnostics` that also registers loader files
/// so content-validation diagnostics (e.g. `ConfigLoaderRequiredFieldMissing`) can
/// fire.  Used for the clean `examples/meta_config/` fixture.
pub(crate) fn check_workspace_no_diagnostics_with_loaders(example_dir: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db_with_loaders(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_issues = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.message
            ));
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.0.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.0.message
            ));
        }
    }

    assert!(
        all_issues.is_empty(),
        "Found {} diagnostic(s) in {}:\n  {}",
        all_issues.len(),
        example_dir,
        all_issues.join("\n  ")
    );
}
