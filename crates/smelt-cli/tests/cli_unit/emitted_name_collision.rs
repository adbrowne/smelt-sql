//! D-02: persisted entities whose `_`-join mapping yields the same
//! `(target_schema, table_name)` pair emit `DuplicateEmittedName` (Error), even
//! when their `smelt.<path>` addresses are distinct. Blocked by the build gate;
//! published by the LSP (covered by the `example_workspaces` LSP gate test).
//!
//! Spec: `docs/specs/architecture.md` §"Default materialization name mapping";
//! `docs/specs/diagnostics.md` (`DuplicateEmittedName`).

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{DiagnosticCode, DiagnosticSeverity, EmittedNameCollisionDiagnostic, Workspace};
use std::path::{Path, PathBuf};

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Load `examples/<name>` and collect every emitted-name-collision diagnostic.
fn emitted_name_collisions_for(example: &str) -> Vec<EmittedNameCollisionDiagnostic> {
    let path = examples_root().join(example);
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap_or_default();

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut out = Vec::new();
    for project in ws.projects(&db).iter().copied() {
        out.extend(
            smelt_db::project_emitted_name_collisions(&db, project)
                .iter()
                .cloned(),
        );
    }
    out
}

/// `staging/orders.sql` (address `staging.orders`) and `staging_orders.sql`
/// (address `staging_orders`) both emit `main.staging_orders` → one Error.
#[test]
fn smelt_build_refuses_emitted_name_collision() {
    let diags = emitted_name_collisions_for("architecture_broken_emitted_name_collision");

    let errors: Vec<&EmittedNameCollisionDiagnostic> = diags
        .iter()
        .filter(|d| d.diagnostic.severity == DiagnosticSeverity::Error)
        .collect();

    assert_eq!(
        errors.len(),
        1,
        "expected exactly one DuplicateEmittedName Error, got {}: {:?}",
        errors.len(),
        diags
            .iter()
            .map(|d| format!(
                "[{:?}] {} emitted={}: {}",
                d.diagnostic.code,
                d.path.display(),
                d.emitted_name,
                d.diagnostic.message
            ))
            .collect::<Vec<_>>()
    );

    let d = errors[0];
    assert_eq!(
        d.diagnostic.code,
        Some(DiagnosticCode::DuplicateEmittedName),
        "expected DuplicateEmittedName code, got {:?}: {}",
        d.diagnostic.code,
        d.diagnostic.message
    );
    assert!(
        d.emitted_name.contains("staging_orders"),
        "emitted_name should reference staging_orders, got: {}",
        d.emitted_name
    );
}

/// Clean workspaces produce zero emitted-name-collision diagnostics.
#[test]
fn clean_workspace_has_no_emitted_name_collision() {
    let diags = emitted_name_collisions_for("ecommerce");
    assert!(
        diags.is_empty(),
        "expected zero emitted-name-collision diagnostics for ecommerce, got: {:?}",
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
