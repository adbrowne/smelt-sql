//! Canonical Salsa ingest path for `smelt_core::workspace::LoadedWorkspace`.
//!
//! See `docs/specs/architecture.md` → "Workspace loading parity rule
//! (CLI ↔ LSP)" for the architectural invariant this module encodes. The CLI's
//! `init_db` and the LSP's `Backend::initialize` BOTH call
//! [`ingest_loaded_workspace`] so the sequence of Salsa input updates stays
//! identical between the two consumers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use smelt_core::workspace::LoadedWorkspace;
use walkdir::WalkDir;

use crate::{Database, ProjectInput, SourceFile};

/// Result of ingesting one [`LoadedWorkspace`]. Callers can collect across
/// multiple projects and call [`Database::set_workspace`] once at the end.
pub struct IngestedProject {
    pub project: ProjectInput,
    pub source_files: Vec<SourceFile>,
    /// Paths in the same order as `source_files`, useful for the LSP's
    /// `tracked_files` list (it stores `PathBuf`, not `SourceFile`).
    pub paths: Vec<PathBuf>,
}

/// Wire a [`LoadedWorkspace`] into the Salsa DB.
///
/// What this does, in order:
///   1. `set_project_input(project_root, sources_text)` for the project.
///   2. `set_source_file(virtual_path, content, project_root)` for each
///      entry in `loaded.sql_files` (models + functions; multi-model
///      frontmatter already expanded by `load_workspace`).
///   3. Walk the project tree for YAML/JSON/TOML data files and register
///      each as a `LoaderFileInput` so `smelt.config.load_yaml` /
///      `smelt.config.load_json` calls in generator files can evaluate.
///      Excludes `smelt.yml`, `smelt.yaml`, `sources.yml`, `sources.yaml`
///      which are handled by separate Salsa inputs.
///
/// Does NOT call `set_workspace` — callers do that once they've ingested
/// every project (the LSP iterates over multiple project roots; the CLI
/// has exactly one).
pub fn ingest_loaded_workspace(db: &mut Database, loaded: &LoadedWorkspace) -> IngestedProject {
    let project = db.set_project_input(loaded.project_root.clone(), loaded.sources_text.clone());

    let mut source_files = Vec::with_capacity(loaded.sql_files.len());
    let mut paths = Vec::with_capacity(loaded.sql_files.len());
    for model in &loaded.sql_files {
        let sf = db.set_source_file(
            model.path.clone(),
            model.content.clone(),
            loaded.project_root.clone(),
        );
        source_files.push(sf);
        paths.push(model.path.clone());
    }

    // Set the active build target from the project's smelt.yml `target:` field.
    // Both the CLI and the LSP use this as the effective target when no `--target`
    // override is supplied, keeping discovery symmetric (Workspace Loading Parity rule).
    // Calling this before register_loader_files_from_disk ensures the Workspace
    // singleton exists so loader files are added to its loader_files list.
    db.set_active_target(loaded.config.target.as_deref().map(Arc::from));

    register_loader_files_from_disk(db, &loaded.project_root);

    IngestedProject {
        project,
        source_files,
        paths,
    }
}

/// Scan `project_root` recursively for YAML/JSON/TOML data files and register
/// each one as a `LoaderFileInput`.
///
/// Lifted from the CLI's `register_loader_files_from_disk` so both the CLI
/// and the LSP populate the loader-file inputs the same way. Without this,
/// `smelt.config.load_yaml(...)` calls in generator files don't resolve in
/// the LSP (one of the asymmetric-discovery bug classes the parity rule
/// addresses).
///
/// Exposed (`pub`) so the LSP — which has its own per-file ingest path via
/// `register_sql_content` for multi-model line-offset tracking — can still
/// register loader files the same way the CLI does.
pub fn register_loader_files_from_disk(db: &mut Database, project_root: &Path) {
    for entry in WalkDir::new(project_root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" && ext != "json" && ext != "toml" {
            continue;
        }
        // Skip the workspace config and sources config files.
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            file_name,
            "smelt.yml" | "smelt.yaml" | "sources.yml" | "sources.yaml"
        ) {
            continue;
        }
        // Compute the workspace-relative path (forward-slash separators).
        let rel = match path.strip_prefix(project_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        db.set_loader_file(Arc::from(rel.as_str()), Arc::from(text.as_str()), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::workspace::load_workspace;

    #[test]
    fn ingest_loaded_workspace_registers_models_and_functions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::create_dir_all(dir.path().join("functions")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();
        std::fs::write(
            dir.path().join("functions").join("inc.sql"),
            "smelt.define inc(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n",
        )
        .unwrap();

        let loaded = load_workspace(dir.path());
        let mut db = Database::default();
        let ingested = ingest_loaded_workspace(&mut db, &loaded);

        assert_eq!(ingested.source_files.len(), 2);
        assert_eq!(ingested.paths.len(), 2);
        // Both files round-trip through `source_file` lookup.
        for path in &ingested.paths {
            assert!(
                db.source_file(path).is_some(),
                "source_file({:?}) should be registered",
                path
            );
        }
        // ProjectInput should be retrievable by root.
        assert!(db.project_input(dir.path()).is_some());
    }

    #[test]
    fn ingest_sets_active_target_from_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\ntargets: {}\ntarget: prod\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();

        let loaded = load_workspace(dir.path());
        let mut db = Database::default();
        ingest_loaded_workspace(&mut db, &loaded);

        let ws = crate::Workspace::try_get(&db).expect("workspace not initialized");
        assert_eq!(ws.active_target(&db).as_deref(), Some("prod"));
    }

    #[test]
    fn ingest_without_config_target_leaves_active_target_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\ntargets: {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();

        let loaded = load_workspace(dir.path());
        let mut db = Database::default();
        ingest_loaded_workspace(&mut db, &loaded);

        let ws = crate::Workspace::try_get(&db).expect("workspace not initialized");
        assert!(ws.active_target(&db).is_none());
    }
}
