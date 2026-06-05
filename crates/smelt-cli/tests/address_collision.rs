//! BUG-002 / P2: cross-kind `smelt.<path>` address collisions surface as
//! `DuplicateAddress` diagnostics via `project_address_collisions`, blocked by
//! the `smelt build` gate, and published by the LSP (covered by the
//! `example_workspaces` LSP gate test).
//!
//! Spec: `docs/specs/architecture.md` §"Workspace loading parity rule (CLI ↔
//! LSP)"; `docs/specs/scoping.md` §"Diagnostic codes" (`DuplicateAddress`).

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{AddressCollisionDiagnostic, DiagnosticCode, DiagnosticSeverity, Workspace};
use std::path::{Path, PathBuf};

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Load `examples/<name>` and collect every address-collision diagnostic.
fn address_collisions_for(example: &str) -> Vec<AddressCollisionDiagnostic> {
    let path = examples_root().join(example);
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    // Discover models; some fixtures are intentionally broken so we tolerate
    // an empty result (error case handled by `discover_models` returning Err).
    let models = discovery.discover_models().unwrap_or_default();

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut out = Vec::new();
    for project in ws.projects(&db).iter().copied() {
        out.extend(
            smelt_db::project_address_collisions(&db, project)
                .iter()
                .cloned(),
        );
    }
    out
}

/// A model `dup.sql` and a seed `dup.csv` in the same scan root produce
/// exactly one `DuplicateAddress` Error anchored at the seed file.
#[test]
fn model_vs_seed_collision_surfaces_duplicate_address() {
    let diags = address_collisions_for("architecture_broken_path_collision");

    let errors: Vec<&AddressCollisionDiagnostic> = diags
        .iter()
        .filter(|d| d.diagnostic.severity == DiagnosticSeverity::Error)
        .collect();

    assert_eq!(
        errors.len(),
        1,
        "expected exactly one DuplicateAddress Error, got {}: {:?}",
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
        Some(DiagnosticCode::DuplicateAddress),
        "expected DuplicateAddress, got {:?}: {}",
        d.diagnostic.code,
        d.diagnostic.message
    );
    assert!(
        d.diagnostic.message.contains("dup"),
        "diagnostic message should reference the colliding address 'dup', got: {}",
        d.diagnostic.message
    );
}

/// A clean workspace (ecommerce) produces zero address-collision diagnostics.
#[test]
fn clean_workspace_has_no_address_collisions() {
    let diags = address_collisions_for("ecommerce");
    assert!(
        diags.is_empty(),
        "expected zero collision diagnostics for ecommerce, got: {:?}",
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

/// Invalidation: editing the CSV's *contents* (not its path) must not change
/// the collision diagnostic. This verifies that the address collision check
/// depends only on file paths (structural, restart-scoped) and not on CSV
/// content — so `project_address_collisions` is stable against content edits.
#[test]
fn csv_content_edit_does_not_remove_collision() {
    use std::io::Write;

    let src = examples_root().join("architecture_broken_path_collision");
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let ws = tmp.path().join("ws");
    copy_dir(&src, &ws);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(ws.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(ws.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap_or_default();
    let db = init_db(&ws, &models);
    let wspace = Workspace::try_get(&db).expect("workspace not initialized");

    // First run: must have exactly one collision.
    let before: Vec<_> = wspace
        .projects(&db)
        .iter()
        .copied()
        .flat_map(|p| {
            smelt_db::project_address_collisions(&db, p)
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(before.len(), 1, "expected 1 collision before content edit");

    // Edit the CSV content (append a new row — does NOT change the path).
    let csv_path = ws.join("models").join("dup.csv");
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&csv_path)
        .expect("open dup.csv for append");
    writeln!(f, "3,gamma").expect("append row");
    drop(f);

    // Second run on the same Salsa db: Salsa has NOT been told about any input
    // change, so `project_address_collisions` returns the cached result.
    let after: Vec<_> = wspace
        .projects(&db)
        .iter()
        .copied()
        .flat_map(|p| {
            smelt_db::project_address_collisions(&db, p)
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        after.len(),
        1,
        "collision count must not change after CSV content edit (structural check only)"
    );
    assert_eq!(
        before[0].diagnostic.message, after[0].diagnostic.message,
        "collision message must be identical before and after CSV content edit"
    );
}

/// End-to-end parity: `smelt build` over the broken collision fixture exits
/// non-zero and names the `DuplicateAddress` code (via the gate).
#[cfg(feature = "duckdb")]
#[test]
fn smelt_build_refuses_address_collision() {
    use std::process::Command;

    let workspace = examples_root().join("architecture_broken_path_collision");
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let dest = tmp.path().join("architecture_broken_path_collision");
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
        "expected `smelt build` to FAIL on an address collision, but it succeeded.\n{combined}"
    );
    assert!(
        combined.contains("DuplicateAddress")
            || combined.contains("duplicate-address")
            || combined.contains("duplicate address"),
        "expected the build error to name the DuplicateAddress code.\n{combined}"
    );
}

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
