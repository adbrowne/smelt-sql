//! Test-only adapter that mimics the old query-group API surface on top of
//! the new `salsa = 0.26` free-function API.
//!
//! The old tests used `db.set_file_text(path, Arc::new(text))` and
//! `db.some_query(path)` — this wrapper provides the same shape so that the
//! tests could be ported mechanically. Production code should prefer the
//! free-function API directly.

#![cfg(test)]
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use crate::*;

/// Test-only Database wrapper that provides the pre-0.26 API shape.
pub struct TestDb {
    pub db: Database,
    project_root: PathBuf,
    files_by_path: std::collections::HashMap<PathBuf, SourceFile>,
    file_order: Vec<PathBuf>,
    project_paths: Vec<PathBuf>,
    yaml_by_project: std::collections::HashMap<PathBuf, String>,
    file_roots: std::collections::HashMap<PathBuf, PathBuf>,
    workspace_dirty: bool,
}

impl Default for TestDb {
    fn default() -> Self {
        Self {
            db: Database::default(),
            project_root: PathBuf::from("."),
            files_by_path: std::collections::HashMap::new(),
            file_order: Vec::new(),
            project_paths: Vec::new(),
            yaml_by_project: std::collections::HashMap::new(),
            file_roots: std::collections::HashMap::new(),
            workspace_dirty: true,
        }
    }
}

impl TestDb {
    fn resolve_root(&self, path: &std::path::Path) -> PathBuf {
        self.file_roots
            .get(path)
            .cloned()
            .unwrap_or_else(|| self.project_root.clone())
    }

    pub fn set_file_text(&mut self, path: PathBuf, text: Arc<String>) {
        let root = self.resolve_root(&path);
        let file = self.db.set_source_file(path.clone(), (*text).clone(), root);
        if !self.files_by_path.contains_key(&path) {
            self.file_order.push(path.clone());
        }
        self.files_by_path.insert(path, file);
        self.workspace_dirty = true;
    }

    pub fn set_all_files(&mut self, files: Arc<Vec<PathBuf>>) {
        self.file_order = (*files).clone();
        // Ensure all entries exist.
        for path in &self.file_order {
            if !self.files_by_path.contains_key(path) {
                let root = self.resolve_root(path);
                let file = self.db.set_source_file(path.clone(), String::new(), root);
                self.files_by_path.insert(path.clone(), file);
            }
        }
        self.workspace_dirty = true;
    }

    pub fn set_file_project_root(&mut self, path: PathBuf, root: PathBuf) {
        self.file_roots.insert(path.clone(), root.clone());
        if let Some(&file) = self.files_by_path.get(&path) {
            let current_text = file.text(&self.db).clone();
            self.db.set_source_file(path, current_text, root);
        }
        self.workspace_dirty = true;
    }

    pub fn set_project_sources_yaml(&mut self, root: PathBuf, yaml: Arc<String>) {
        self.db.set_project_input(root.clone(), (*yaml).clone());
        self.yaml_by_project.insert(root.clone(), (*yaml).clone());
        if !self.project_paths.contains(&root) {
            self.project_paths.push(root);
        }
        self.workspace_dirty = true;
    }

    pub fn set_all_project_roots(&mut self, roots: Arc<Vec<PathBuf>>) {
        self.project_paths = (*roots).clone();
        for r in &self.project_paths {
            if !self.yaml_by_project.contains_key(r) {
                self.db.set_project_input(r.clone(), String::new());
                self.yaml_by_project.insert(r.clone(), String::new());
            }
        }
        self.workspace_dirty = true;
        self.sync_workspace();
    }

    pub fn sync_workspace(&mut self) -> Workspace {
        if self.workspace_dirty {
            let files: Vec<SourceFile> = self
                .file_order
                .iter()
                .filter_map(|p| self.files_by_path.get(p).copied())
                .collect();
            let projects: Vec<ProjectInput> = self
                .project_paths
                .iter()
                .filter_map(|p| self.db.project_input(p))
                .collect();
            self.db.set_workspace(files, projects);
            self.workspace_dirty = false;
        }
        self.db.workspace()
    }

    fn file(&self, path: &std::path::Path) -> SourceFile {
        *self
            .files_by_path
            .get(path)
            .unwrap_or_else(|| panic!("no SourceFile registered for {:?}", path))
    }

    // --------- Query wrappers matching the old trait method surface --------

    pub fn file_text(&self, path: PathBuf) -> Arc<String> {
        Arc::new(self.file(&path).text(&self.db).clone())
    }

    pub fn parse_file(&mut self, path: PathBuf) -> Arc<smelt_parser::Parse> {
        let file = self.file(&path);
        Arc::new(parse_file(&self.db, file).clone())
    }

    pub fn parse_model(&mut self, path: PathBuf) -> Option<Arc<Model>> {
        let file = self.file(&path);
        parse_model(&self.db, file)
    }

    pub fn model_sources(&mut self, path: PathBuf) -> Arc<Vec<SourceLocation>> {
        let file = self.file(&path);
        model_sources(&self.db, file)
    }

    pub fn resolve_source(
        &mut self,
        project_root: PathBuf,
        source_name: String,
        table_name: String,
    ) -> Option<SourceTableDef> {
        self.sync_workspace();
        let p = self
            .db
            .project_input(&project_root)
            .expect("project not registered");
        resolve_source(&self.db, p, source_name, table_name)
    }

    pub fn file_diagnostics(&mut self, path: PathBuf) -> Arc<Vec<Diagnostic>> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        Arc::new(file_diagnostics(&self.db, ws, file))
    }

    pub fn model_schema(&mut self, path: PathBuf) -> Arc<ModelSchema> {
        self.sync_workspace();
        let file = self.file(&path);
        model_schema(&self.db, file)
    }

    pub fn available_columns(&mut self, path: PathBuf) -> Arc<Vec<Column>> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        available_columns(&self.db, ws, file)
    }

    pub fn typed_model_schema(&mut self, path: PathBuf) -> Arc<ModelSchema> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        typed_model_schema(&self.db, ws, file)
    }

    pub fn type_context(&mut self, path: PathBuf) -> Arc<TypeContext> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        type_context(&self.db, ws, file)
    }

    pub fn resolved_model_schema(&mut self, path: PathBuf) -> Arc<ResolvedSchema> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        resolved_model_schema(&self.db, ws, file)
    }

    pub fn model_input_constraints(&mut self, path: PathBuf) -> Arc<Vec<InputConstraint>> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        model_input_constraints(&self.db, ws, file)
    }

    pub fn model_function_type(&mut self, path: PathBuf) -> Arc<schema::ModelFunctionType> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        model_function_type(&self.db, ws, file)
    }

    pub fn type_diagnostics(&mut self, path: PathBuf) -> Arc<Vec<Diagnostic>> {
        let ws = self.sync_workspace();
        let file = self.file(&path);
        let diags: Vec<Diagnostic> =
            check_type_diagnostics::accumulated::<DiagnosticAcc>(&self.db, ws, file)
                .into_iter()
                .map(|d| d.0.clone())
                .collect();
        Arc::new(diags)
    }
}
