//! `examples/broken/models/sources/retention_*.yml` fixtures each produce
//! exactly one `MalformedSource` diagnostic, and no other `examples/broken`
//! file regresses. `models/retention_exceeded.sql` (a well-formed
//! `retention: '7 days'` source read with a 30-day lookback) produces
//! exactly one `SourceRetentionExceeded`.
//!
//! Spec: `docs/specs/sources.md` §"Retention refusal", §"Diagnostic codes".
//! Plan: `docs/outcomes/20260906-trimmed-history-sources/phases/02-plan.md`,
//! `phases/04-plan.md`.

use crate::support::*;

fn broken_source_diagnostics() -> Vec<smelt_db::SourceDiagnostic> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/broken");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut out = Vec::new();
    for project in ws.projects(&db).iter().copied() {
        out.extend(
            smelt_db::project_source_diagnostics(&db, project)
                .iter()
                .cloned(),
        );
    }
    out
}

fn assert_exactly_one(
    diags: &[smelt_db::SourceDiagnostic],
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.path.ends_with(expected_file))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic for '{}', got {}: {:?}",
        expected_file,
        matches.len(),
        diags
            .iter()
            .map(|d| format!(
                "[{:?}] {}: {}",
                d.diagnostic.code,
                d.path.display(),
                d.diagnostic.message
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        matches[0].diagnostic.code,
        Some(expected_code),
        "expected {:?} for '{}', got {:?}: {}",
        expected_code,
        expected_file,
        matches[0].diagnostic.code,
        matches[0].diagnostic.message
    );
}

#[test]
fn broken_workspace_retention_fixtures() {
    let diags = broken_source_diagnostics();

    assert_exactly_one(
        &diags,
        "sources/retention_bad_interval.yml",
        smelt_db::DiagnosticCode::MalformedSource,
    );
    assert_exactly_one(
        &diags,
        "sources/retention_zero.yml",
        smelt_db::DiagnosticCode::MalformedSource,
    );
    assert_exactly_one(
        &diags,
        "sources/retention_unclocked.yml",
        smelt_db::DiagnosticCode::MalformedSource,
    );

    // No file outside the three fixtures above should carry a MalformedSource
    // diagnostic attributable to retention — the rest of examples/broken/ is
    // untouched by this feature.
    let unexpected: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.diagnostic.code,
                Some(smelt_db::DiagnosticCode::MalformedSource)
            )
        })
        .filter(|d| d.diagnostic.message.to_lowercase().contains("retention"))
        .filter(|d| {
            let p = d.path.to_string_lossy();
            !(p.ends_with("sources/retention_bad_interval.yml")
                || p.ends_with("sources/retention_zero.yml")
                || p.ends_with("sources/retention_unclocked.yml"))
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected retention diagnostics: {:?}",
        unexpected
            .iter()
            .map(|d| format!("{}: {}", d.path.display(), d.diagnostic.message))
            .collect::<Vec<_>>()
    );
}

/// Phase 4 TDD: `examples/broken/models/retention_exceeded.sql` reaches 30
/// days back into `sources/retention_exceeded_events.yml`'s well-formed
/// `retention: '7 days'` — refuses with exactly one `SourceRetentionExceeded`
/// diagnostic, and no other file in the shared `examples/broken/` workspace
/// carries that code (`docs/specs/model_properties.md` §"Reach versus
/// retained history").
#[test]
fn broken_retention_exceeded_example_reports_the_named_code() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticCode, Workspace};

    let expected_file = "models/retention_exceeded.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/broken");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

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
        let is_target = rel.replace('\\', "/").ends_with(expected_file);

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if d.code != Some(DiagnosticCode::SourceRetentionExceeded) {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if d.0.code != Some(DiagnosticCode::SourceRetentionExceeded) {
                continue;
            }
            if is_target {
                target.push(d.0.clone());
            } else {
                other.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other.is_empty(),
        "expected zero SourceRetentionExceeded diagnostics from files other than \
         '{expected_file}', got {}:\n  {}",
        other.len(),
        other
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        target.len(),
        1,
        "expected exactly one SourceRetentionExceeded diagnostic from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
