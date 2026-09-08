//! `examples/broken/models/sources/step_*.yml` fixtures each produce exactly
//! their expected external-step diagnostic, and no other `examples/broken`
//! file regresses.
//!
//! Spec: `docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)". Plan: `docs/outcomes/20260906-external-dag-steps/phases/02-plan.md`.

use crate::support::*;

fn broken_step_diagnostics() -> Vec<smelt_db::SourceDiagnostic> {
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
fn broken_workspace_external_step_fixtures() {
    let diags = broken_step_diagnostics();

    assert_exactly_one(
        &diags,
        "sources/step_missing_produces.yml",
        smelt_db::DiagnosticCode::MalformedExternalStep,
    );
    assert_exactly_one(
        &diags,
        "sources/step_bad_command.yml",
        smelt_db::DiagnosticCode::MalformedExternalStep,
    );
    assert_exactly_one(
        &diags,
        "sources/step_columns_present.yml",
        smelt_db::DiagnosticCode::MalformedExternalStep,
    );
    assert_exactly_one(
        &diags,
        "sources/step_bad_cadence.yml",
        smelt_db::DiagnosticCode::MalformedExternalStep,
    );
    assert_exactly_one(
        &diags,
        "sources/step_unknown_source.yml",
        smelt_db::DiagnosticCode::MalformedExternalStep,
    );
    assert_exactly_one(
        &diags,
        "sources/step_dup_producer_b.yml",
        smelt_db::DiagnosticCode::SourceProducerConflict,
    );

    // `step_dup_producer_a.yml` itself is well-formed and is not the conflict
    // anchor — it must not appear in the diagnostic set at all.
    assert!(
        diags
            .iter()
            .all(|d| !d.path.ends_with("sources/step_dup_producer_a.yml")),
        "step_dup_producer_a.yml should not itself carry a diagnostic: {:?}",
        diags
            .iter()
            .map(|d| format!("{}: {}", d.path.display(), d.diagnostic.message))
            .collect::<Vec<_>>()
    );

    // No file outside the six fixtures above should carry an external-step
    // diagnostic — the rest of examples/broken/ is untouched by this feature.
    let unexpected: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.diagnostic.code,
                Some(smelt_db::DiagnosticCode::MalformedExternalStep)
                    | Some(smelt_db::DiagnosticCode::SourceProducerConflict)
            )
        })
        .filter(|d| {
            let p = d.path.to_string_lossy();
            !(p.ends_with("sources/step_missing_produces.yml")
                || p.ends_with("sources/step_bad_command.yml")
                || p.ends_with("sources/step_columns_present.yml")
                || p.ends_with("sources/step_bad_cadence.yml")
                || p.ends_with("sources/step_unknown_source.yml")
                || p.ends_with("sources/step_dup_producer_b.yml"))
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected external-step diagnostics: {:?}",
        unexpected
            .iter()
            .map(|d| format!("{}: {}", d.path.display(), d.diagnostic.message))
            .collect::<Vec<_>>()
    );
}
