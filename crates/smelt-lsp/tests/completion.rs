//! Phase 2c — completion tests for `smelt.<path>` context.
//!
//! Test 6: `completes_workspace_paths_after_smelt_dot`
//!   - Asserts that `determine_completion_context` returns `SmeltPath` when
//!     the cursor is positioned after a `smelt.` prefix in a FROM clause.
//!   - Asserts that workspace file listing produces completions that include
//!     the path-form label for registered models.
//!
//! NOTE: `determine_completion_context` must be made `pub` (or at least
//! `pub(crate)` with `#[cfg(test)]` re-export) as part of the Phase 2c
//! implementation. Until then, these tests will fail to compile, which is
//! the intended "red" state.

use std::path::PathBuf;

use smelt_db::{Database, SourceFile, Workspace};
use smelt_lsp::{determine_completion_context, CompletionContext};

// ---------------------------------------------------------------------------
// Workspace helper (mirrors pattern from other lsp tests)
// ---------------------------------------------------------------------------

struct CompletionTestWs {
    _temp: tempfile::TempDir,
    pub db: Database,
    project_root: PathBuf,
    files: Vec<PathBuf>,
}

impl CompletionTestWs {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().to_path_buf();
        let mut db = Database::default();
        db.set_project_input(project_root.clone(), String::new());
        db.set_workspace(Vec::new(), Vec::new());
        Self {
            _temp: temp,
            db,
            project_root,
            files: Vec::new(),
        }
    }

    fn add_sql(&mut self, rel_path: &str, sql: &str) -> PathBuf {
        let path = self.project_root.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, sql).unwrap();
        self.db
            .set_source_file(path.clone(), sql.to_string(), self.project_root.clone());
        self.files.push(path.clone());
        self.sync();
        path
    }

    fn sync(&mut self) {
        let files: Vec<SourceFile> = self
            .files
            .iter()
            .filter_map(|p| self.db.source_file(p))
            .collect();
        let projects = self
            .db
            .project_input(&self.project_root)
            .into_iter()
            .collect::<Vec<_>>();
        self.db.set_workspace(files, projects);
    }

    fn workspace(&self) -> Workspace {
        Workspace::try_get(&self.db).expect("workspace set")
    }
}

// ---------------------------------------------------------------------------
// Test 6: completes_workspace_paths_after_smelt_dot
// ---------------------------------------------------------------------------

/// `determine_completion_context` must return `SmeltPath` when the cursor is
/// immediately after a `smelt.` prefix in a FROM position.
#[test]
fn determine_context_returns_smelt_path_after_prefix() {
    let text = "SELECT * FROM smelt.";
    let ctx = determine_completion_context(text, text.len());
    assert!(
        matches!(ctx, CompletionContext::SmeltPath),
        "cursor after 'smelt.' must yield SmeltPath context; got: {ctx:?}"
    );
}

/// A partial path like `smelt.models.` also triggers `SmeltPath` completion.
#[test]
fn determine_context_returns_smelt_path_for_partial() {
    let text = "SELECT * FROM smelt.models.";
    let ctx = determine_completion_context(text, text.len());
    assert!(
        matches!(ctx, CompletionContext::SmeltPath),
        "cursor after 'smelt.models.' must yield SmeltPath context; got: {ctx:?}"
    );
}

/// Entry-point test required by Phase 2c checklist. Covers context detection
/// and workspace path listing together.
#[test]
fn completes_workspace_paths_after_smelt_dot() {
    // Context detection
    let text = "SELECT * FROM smelt.";
    let ctx = determine_completion_context(text, text.len());
    assert!(
        matches!(ctx, CompletionContext::SmeltPath),
        "cursor after 'smelt.' must yield SmeltPath; got: {ctx:?}"
    );
    // Workspace path listing
    let mut ws = CompletionTestWs::new();
    ws.add_sql("models/users.sql", "SELECT 1 AS user_id\n");
    let workspace = ws.workspace();
    let all_files = workspace.files(&ws.db).clone();
    let project_root = ws.project_root.clone();
    let labels: Vec<String> = all_files
        .iter()
        .filter_map(|f| {
            let file_path = f.path(&ws.db);
            let rel = file_path.strip_prefix(&project_root).ok()?;
            let parent = rel.parent()?;
            let mut segs: Vec<String> = parent
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())?;
            segs.push(stem);
            Some(format!("smelt.{}", segs.join(".")))
        })
        .collect();
    assert!(
        labels.iter().any(|l| l == "smelt.models.users"),
        "path labels must include 'smelt.models.users'; got: {labels:?}"
    );
}

/// Workspace path listing: registered model files should be expressible as
/// `smelt.models.<name>` completion items. Verify the path-tuple logic is
/// correct.
#[test]
fn workspace_path_list_includes_registered_models() {
    let mut ws = CompletionTestWs::new();
    ws.add_sql("models/users.sql", "SELECT 1 AS user_id\n");
    ws.add_sql("models/orders.sql", "SELECT 1 AS order_id\n");

    let workspace = ws.workspace();

    // Enumerate files and compute their path tuples.
    let all_files = workspace.files(&ws.db).clone();
    let project_root = ws.project_root.clone();

    let mut path_labels: Vec<String> = all_files
        .iter()
        .filter_map(|f| {
            let file_path = f.path(&ws.db);
            let rel = file_path.strip_prefix(&project_root).ok()?;
            let parent = rel.parent()?;
            let mut segments: Vec<String> = parent
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())?;
            segments.push(stem);
            Some(format!("smelt.{}", segments.join(".")))
        })
        .collect();

    path_labels.sort();

    assert!(
        path_labels.iter().any(|l| l == "smelt.models.users"),
        "path labels must include 'smelt.models.users'; got: {path_labels:?}"
    );
    assert!(
        path_labels.iter().any(|l| l == "smelt.models.orders"),
        "path labels must include 'smelt.models.orders'; got: {path_labels:?}"
    );
}
