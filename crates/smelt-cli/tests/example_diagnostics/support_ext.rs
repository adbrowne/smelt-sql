use crate::support::*;

/// Helper: loads `example_dir` as a workspace (with loader files registered),
/// checks that exactly one Phase E1 diagnostic fires for the file ending in
/// `expected_file`, and that no other file emits Phase E1 codes.
pub(crate) fn check_workspace_emits_exactly_one_phase_e1_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
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

    let mut target_phase_e1: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_e1: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_e1 = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_E1_CODES.contains(c))
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
            if !is_phase_e1(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_e1.push(d.clone());
            } else {
                other_phase_e1.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_e1(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_e1.push(d.0.clone());
            } else {
                other_phase_e1.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_e1.is_empty(),
        "expected zero Phase E1 diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_e1.len(),
        other_phase_e1
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_e1.len(),
        1,
        "expected exactly 1 Phase E1 diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_e1.len(),
        target_phase_e1
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_e1[0].code,
        Some(expected_code),
        "expected Phase E1 diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_e1[0].code,
        target_phase_e1[0].message
    );
}

/// Helper: loads `example_dir`, asserts exactly one Phase E2 diagnostic fires
/// for the file ending in `expected_file`, and that no other file emits E2 codes.
pub(crate) fn check_workspace_emits_exactly_one_phase_e2_diagnostic(
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

    let mut target_e2: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_e2: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_e2 = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_E2_CODES.contains(c))
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
            if !is_e2(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_e2.push(d.clone());
            } else {
                other_e2.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_e2(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_e2.push(d.0.clone());
            } else {
                other_e2.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_e2.is_empty(),
        "expected zero Phase E2 diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_e2.len(),
        other_e2
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_e2.len(),
        1,
        "expected exactly 1 Phase E2 diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_e2.len(),
        target_e2
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_e2[0].code,
        Some(expected_code),
        "expected Phase E2 code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_e2[0].code,
        target_e2[0].message
    );
}

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `TimeseriesRequiredForPartitionGrain` or `MalformedTimeseries` diagnostic fires
/// for the file ending in `expected_file`, and no such diagnostic fires in any
/// other file in the workspace.
pub(crate) fn check_workspace_emits_timeseries_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    const TIMESERIES_CODES: &[smelt_db::DiagnosticCode] = &[
        smelt_db::DiagnosticCode::TimeseriesRequiredForPartitionGrain,
        smelt_db::DiagnosticCode::MalformedTimeseries,
    ];

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

    let mut target_ts: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_ts: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_ts = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| TIMESERIES_CODES.contains(c))
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
            if !is_ts(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_ts.push(d.clone());
            } else {
                other_ts.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_ts(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_ts.push(d.0.clone());
            } else {
                other_ts.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_ts.is_empty(),
        "expected zero timeseries diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_ts.len(),
        other_ts
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_ts.len(),
        1,
        "expected exactly 1 timeseries diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_ts.len(),
        target_ts
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_ts[0].code,
        Some(expected_code),
        "expected timeseries diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_ts[0].code,
        target_ts[0].message
    );
}

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `KeyedForbidsTimeseries` diagnostic fires for the file ending in
/// `expected_file`, and no such diagnostic fires in any other file in the
/// workspace.
pub(crate) fn check_workspace_emits_keyed_frontmatter_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) -> smelt_db::Diagnostic {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    const KEYED_FRONTMATTER_CODES: &[smelt_db::DiagnosticCode] =
        &[smelt_db::DiagnosticCode::KeyedForbidsTimeseries];

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

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_keyed_frontmatter = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| KEYED_FRONTMATTER_CODES.contains(c))
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
            if !is_keyed_frontmatter(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_keyed_frontmatter(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero keyed frontmatter diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 keyed frontmatter diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(expected_code),
        "expected keyed frontmatter diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );

    target_diags.into_iter().next().unwrap()
}

/// Helper: loads `example_dir` as a workspace and checks that exactly one
/// `AliasColumnArityMismatch` diagnostic fires for the file ending in
/// `expected_file`, and that no other file in the workspace emits that code.
pub(crate) fn check_workspace_emits_exactly_one_alias_arity_mismatch(
    example_dir: &str,
    expected_file: &str,
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

    let is_target_code = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code == Some(&smelt_db::DiagnosticCode::AliasColumnArityMismatch)
    };

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

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
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero AliasColumnArityMismatch diagnostics from files other than '{}' in {}, \
         got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 AliasColumnArityMismatch from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(smelt_db::DiagnosticCode::AliasColumnArityMismatch),
        "expected AliasColumnArityMismatch from '{}' in {}, got {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// Helper: loads `example_dir`, asserts exactly one emission-body diagnostic fires
/// for the file ending in `expected_file`, and that no other file emits the target code.
pub(crate) fn check_workspace_emits_exactly_one_emission_body_diagnostic(
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

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_target_code =
        |code: Option<&smelt_db::DiagnosticCode>| -> bool { code == Some(&expected_code) };

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
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero {:?} diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_code,
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 {:?} diagnostic from '{}' in {}, got {}:\n  {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(expected_code),
        "expected {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `EventTimeColumnNotVisibleAtOuterSelect` diagnostic fires for the file
/// ending in `expected_file`, and no such diagnostic fires in any other file
/// in the workspace.
pub(crate) fn check_workspace_emits_event_time_not_visible_diagnostic(
    example_dir: &str,
    expected_file: &str,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let target_code = smelt_db::DiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect;

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

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_target_code = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| *c == target_code)
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
        let is_target_file = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target_file {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target_file {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero EventTimeColumnNotVisibleAtOuterSelect diagnostics from files other \
         than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 EventTimeColumnNotVisibleAtOuterSelect diagnostic from '{}' in \
         {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(target_code),
        "unexpected diagnostic code from '{}' in {}: {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}
