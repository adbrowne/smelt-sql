use crate::support::*;
use crate::support_ext::*;

/// BUG-006 regression: `examples/timeseries_broken_cumulative_with_timeseries/` produces
/// exactly one `KeyedForbidsTimeseries` diagnostic from
/// `models/cumulative_with_timeseries.sql`.
///
/// Before the fix, `validate_timeseries` returned `KeyedForbidsTimeseries`
/// but `file_diagnostics` silently dropped it (`_ => None` in the match block),
/// so the LSP showed no error even though keyed models must not declare
/// `timeseries:` without key temporal locality (`incremental_shapes.md` §"Key-grain output shape").
///
/// The diagnostic now comes from the key-temporal-locality gate in plan
/// derivation (`smelt_logical::maintenance::locality::establish_locality`),
/// not frontmatter validation — the message must name all three routes and
/// the nearest missing fact
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
/// time-partitioned output)").
#[test]
fn timeseries_broken_cumulative_with_timeseries() {
    let diag = check_workspace_emits_keyed_frontmatter_diagnostic(
        "examples/timeseries_broken_cumulative_with_timeseries",
        "models/cumulative_with_timeseries.sql",
        smelt_db::DiagnosticCode::KeyedForbidsTimeseries,
    );
    let message = diag.message.to_lowercase();
    for expected in ["key-embedded", "key-determined", "recurrence-bounded"] {
        assert!(
            message.contains(expected),
            "expected the KeyedForbidsTimeseries message to name route '{expected}': {}",
            diag.message
        );
    }
    assert!(
        diag.message.contains("Nearest missing fact"),
        "expected the KeyedForbidsTimeseries message to name the nearest missing fact: {}",
        diag.message
    );
}

/// Verifies that an unrecognized type name nested in a struct field position in
/// a function's return or parameter annotation emits `UnknownStructFieldType`
/// at the individual field's type-ref span (not the whole annotation).
///
/// The broken workspace `examples/functions_broken_struct_field_type/` contains
/// a `smelt.define` with `-> Expr<Struct<{a: Integer, b: Bogus}>>` where `Bogus`
/// is not a known DataType. The declaration must be flagged; no other file in
/// the workspace may produce this code.
#[test]
fn functions_broken_struct_field_type_emits_unknown_struct_field_type() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/functions_broken_struct_field_type";
    let expected_file = "functions/broken_struct.sql";

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
        code == Some(&smelt_db::DiagnosticCode::UnknownStructFieldType)
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
        "expected zero UnknownStructFieldType diagnostics from files other than '{}' in {}, \
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
        "expected exactly 1 UnknownStructFieldType from '{}' in {}, got {}:\n  {}",
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
        Some(smelt_db::DiagnosticCode::UnknownStructFieldType),
        "expected UnknownStructFieldType from '{}' in {}, got {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}
