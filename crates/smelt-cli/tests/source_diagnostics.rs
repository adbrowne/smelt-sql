//! BUG-032 / P2c: malformed per-entity source YAML must surface as a
//! `MalformedSource` diagnostic through the analyzer surface (and therefore be
//! refused by the `smelt build` diagnostic-parity gate), instead of being
//! silently dropped by source discovery and failing the build downstream with a
//! misleading "schema does not exist".
//!
//! Spec: `docs/specs/architecture.md` §"Diagnostic parity rule (analysis ↔
//! build)"; `docs/specs/sources.md` §"Diagnostic codes".

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{DiagnosticCode, DiagnosticSeverity, SourceDiagnostic, Workspace};
use std::path::{Path, PathBuf};

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Load `examples/<name>` as a workspace (Salsa-direct path, same as
/// `example_diagnostics.rs`) and collect every per-entity source diagnostic
/// across the workspace's projects.
fn source_diagnostics_for(example: &str) -> Vec<SourceDiagnostic> {
    let path = examples_root().join(example);
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    if let Ok(function_files) = discovery.discover_function_files() {
        models.extend(function_files);
    }

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

/// A `materialization:`-bearing per-entity source (forbidden on sources)
/// surfaces exactly one `MalformedSource` Error anchored at the offending
/// `.yml`. Today (before P2c) discovery drops the file and the analyzer is
/// silent — this assertion is red without the source-diagnostics producer.
#[test]
fn malformed_source_surfaces_diagnostic() {
    let diags = source_diagnostics_for("sources_broken_malformed");

    let errors: Vec<&SourceDiagnostic> = diags
        .iter()
        .filter(|d| d.diagnostic.severity == DiagnosticSeverity::Error)
        .collect();

    assert_eq!(
        errors.len(),
        1,
        "expected exactly one source Error, got {}: {:?}",
        errors.len(),
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

    let d = errors[0];
    assert_eq!(
        d.diagnostic.code,
        Some(DiagnosticCode::MalformedSource),
        "expected MalformedSource, got {:?}: {}",
        d.diagnostic.code,
        d.diagnostic.message
    );
    assert!(
        d.path.ends_with("sources/raw/orders.yml"),
        "diagnostic should be anchored at the offending source file, got {}",
        d.path.display()
    );
}

/// A workspace whose per-entity sources are all valid produces zero source
/// diagnostics (the producer does not flag well-formed sources).
#[test]
fn valid_sources_stay_clean() {
    let diags = source_diagnostics_for("ecommerce");
    assert!(
        diags.is_empty(),
        "expected zero source diagnostics for ecommerce, got: {:?}",
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
}

/// End-to-end parity: `smelt build` over the malformed-source fixture exits
/// non-zero and names the `MalformedSource` code at the diagnostic-parity gate,
/// where today it builds at exit 0 (the source has no consumer).
#[cfg(feature = "duckdb")]
#[test]
fn smelt_build_refuses_malformed_source() {
    use std::process::Command;

    let workspace = examples_root().join("sources_broken_malformed");
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let dest = tmp.path().join("sources_broken_malformed");
    copy_dir(&workspace, &dest);

    let out = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("build")
        .args(["--project-dir", dest.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn `smelt build`");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !out.status.success(),
        "expected `smelt build` to FAIL on a malformed source, but it succeeded.\n{combined}"
    );
    assert!(
        combined.contains("MalformedSource"),
        "expected the build error to name the MalformedSource code.\n{combined}"
    );
}

#[cfg(feature = "duckdb")]
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}
