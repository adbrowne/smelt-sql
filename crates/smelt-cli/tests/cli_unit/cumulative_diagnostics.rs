//! Diagnostic-parity regression: the `refresh: keyed` classifier's
//! rejections are surfaced through the analysis layer (`file_diagnostics`),
//! not only at `smelt run`/`build`.
//!
//! Closes BUG-011 (`docs/bug-hunt/2026-05-30-findings.md`): a malformed
//! keyed model used to be LSP-clean — `file_diagnostics` reported nothing
//! while the build refused it. The uniform rule → diagnostics interface
//! (`architecture.md` §"Planner scope") now routes the classifier through
//! `file_diagnostics`, so the editor and the build reach the same verdict.

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{DiagnosticCode, Workspace};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/cumulative_classifier_gate")
}

/// Build a Salsa db over the cumulative_classifier_gate fixture and return the
/// `file_diagnostics` for the model whose file stem is `model_stem`.
fn diagnostics_for(model_stem: &str) -> Vec<smelt_db::Diagnostic> {
    let path = fixture_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();
    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let model = models
        .iter()
        .find(|m| m.path.file_stem().and_then(|s| s.to_str()) == Some(model_stem))
        .unwrap_or_else(|| panic!("model `{model_stem}` not found in fixture"));
    let file = db.source_file(&model.path).expect("source file");
    smelt_db::file_diagnostics(&db, ws, file)
}

/// A `STRING_AGG` keyed model is flagged with `KeyedUnknownCombiner`
/// at the analysis layer (where it used to be diagnostic-clean).
#[test]
fn unknown_combiner_surfaces_in_file_diagnostics() {
    let diags = diagnostics_for("edges_bad_aggregator");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::KeyedUnknownCombiner)
                && d.severity == smelt_db::DiagnosticSeverity::Error),
        "expected a KeyedUnknownCombiner Error from file_diagnostics; got: {:?}",
        diags
            .iter()
            .map(|d| (d.severity, d.code, d.message.clone()))
            .collect::<Vec<_>>()
    );
}

/// A well-formed keyed model stays diagnostic-clean (guards against
/// over-rejection, including a spurious `KeyedSnapshotPostureUnsupported` when
/// the driving-source timeseries map is built correctly from the fixture).
#[test]
fn valid_keyed_is_clean_in_file_diagnostics() {
    let diags = diagnostics_for("edges_valid");
    assert!(
        diags.is_empty(),
        "valid keyed model must be diagnostic-clean; got: {:?}",
        diags
            .iter()
            .map(|d| (d.severity, d.code, d.message.clone()))
            .collect::<Vec<_>>()
    );
}
