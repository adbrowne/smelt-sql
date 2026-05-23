//! Thin helpers mapping path-based LSP operations onto the salsa 0.26 API.
//!
//! The `smelt_db::Database` exposes inputs as structs (`SourceFile`,
//! `ProjectInput`) rather than keyed queries. The LSP still thinks in terms of
//! file paths, so these helpers look up the right input struct and call the
//! matching free-function tracked query.

use std::path::{Path, PathBuf};

use smelt_db::{
    check_type_diagnostics, file_diagnostics, Database, Diagnostic as DbDiagnostic, DiagnosticAcc,
    ProjectInput, SourceFile, Workspace,
};

/// Look up the `SourceFile` input for `path`, returning `None` if not
/// registered yet.
pub(crate) fn lookup_file(db: &Database, path: &Path) -> Option<SourceFile> {
    db.source_file(path)
}

/// All source files currently registered in the workspace.
pub(crate) fn workspace_files(db: &Database) -> Vec<SourceFile> {
    match Workspace::try_get(db) {
        Some(ws) => ws.files(db).clone(),
        None => Vec::new(),
    }
}

/// All known file paths.
pub(crate) fn all_file_paths(db: &Database) -> Vec<PathBuf> {
    workspace_files(db)
        .into_iter()
        .map(|f| f.path(db).clone())
        .collect()
}

/// Look up a `ProjectInput` by its root path.
pub(crate) fn lookup_project(db: &Database, root: &Path) -> Option<ProjectInput> {
    db.project_input(root)
}

/// Resolve a model name to the file that defines it (within `project`).
///
/// Project-scoped per the project isolation rule — callers must pass the
/// project containing the file under analysis. `project_root` is the
/// project root path on disk (as recorded on the `SourceFile` input).
pub(crate) fn resolve_ref_path(
    db: &Database,
    project_root: &Path,
    model_name: &str,
) -> Option<PathBuf> {
    let ws = Workspace::try_get(db)?;
    let project = lookup_project(db, project_root)?;
    smelt_db::resolve_ref(db, ws, project, model_name.to_string()).map(|f| f.path(db).clone())
}

/// Shorthand for calling the `file_diagnostics` query given a file path.
pub(crate) fn diagnostics_for(db: &Database, path: &Path) -> Vec<DbDiagnostic> {
    let Some(file) = lookup_file(db, path) else {
        return Vec::new();
    };
    let ws = match Workspace::try_get(db) {
        Some(w) => w,
        None => return Vec::new(),
    };
    let mut diags = file_diagnostics(db, ws, file);
    diags.extend(
        check_type_diagnostics::accumulated::<DiagnosticAcc>(db, ws, file)
            .into_iter()
            .map(|d| d.0.clone()),
    );
    diags
}

/// Project root recorded on the `SourceFile` input for `path`.
pub(crate) fn file_project_root(db: &Database, path: &Path) -> PathBuf {
    lookup_file(db, path)
        .map(|f| f.project_root(db).clone())
        .unwrap_or_default()
}

/// File text for `path`; returns empty string if the file isn't registered.
pub(crate) fn file_text(db: &Database, path: &Path) -> String {
    lookup_file(db, path)
        .map(|f| f.text(db).clone())
        .unwrap_or_default()
}

/// Raw sources.yml text for the project rooted at `root`.
pub(crate) fn project_sources_yaml(db: &Database, root: &Path) -> String {
    lookup_project(db, root)
        .map(|p| p.sources_yaml(db).clone())
        .unwrap_or_default()
}
