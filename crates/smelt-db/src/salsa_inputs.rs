//! Salsa scaffolding: the crate's inputs (`SourceFile`, `ProjectInput`,
//! `Workspace`, `DeployedSchemaInput`, `LoaderFileInput`), the
//! `DiagnosticAcc` accumulator, and the `Database` with its path-keyed
//! registries for looking those inputs up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use salsa::Setter;

use crate::*;

/// A source file tracked by Salsa. Holds the file's current text and the
/// project root it belongs to. Looked up by path via the Database's
/// internal registry (`Database::source_file`).
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub text: String,
    #[returns(ref)]
    pub project_root: PathBuf,
}

/// A project-level input storing the raw `sources.yml` text for the project.
/// Looked up by project root path via `Database::project_input`.
#[salsa::input]
pub struct ProjectInput {
    #[returns(ref)]
    pub root: PathBuf,
    #[returns(ref)]
    pub sources_yaml: String,
    /// Raw text of the workspace's `smelt.yml` file. Empty string when not
    /// loaded (treated as no unstable flags set). Updated by the LSP whenever
    /// the file changes on disk, keeping Salsa's change detection valid.
    #[returns(ref)]
    pub smelt_yml_text: String,
}

/// Workspace-level singleton input tracking the full set of files and projects.
/// Queries that need to enumerate everything (e.g. `all_models`, `resolve_ref`)
/// read from this singleton so they're invalidated when the file set changes.
#[salsa::input(singleton)]
pub struct Workspace {
    #[returns(ref)]
    pub files: Vec<SourceFile>,
    #[returns(ref)]
    pub projects: Vec<ProjectInput>,
    /// All registered loader-file inputs. Kept here so that Salsa-tracked queries
    /// (which receive `&dyn salsa::Database`, not the concrete `Database`) can
    /// enumerate registered loader files without downcasting.
    #[returns(ref)]
    pub loader_files: Vec<LoaderFileInput>,
    /// The active build target (e.g. `"prod"`, `"staging"`). When `Some`, the
    /// loader resolution path dispatches to `loader_resolved_value_with_overlay`
    /// using `<basename>.<target>.<ext>` overlay files. `None` means base-only
    /// resolution. Set from the `smelt.yml` `target:` field (default) or via
    /// the `--target` CLI flag (override). Both CLI and LSP read the same
    /// config-derived default to keep discovery symmetric.
    pub active_target: Option<Arc<str>>,
    /// All registered deployed-schema snapshot inputs. Kept here so that
    /// Salsa-tracked queries (which receive `&dyn salsa::Database`, not the
    /// concrete `Database`) can enumerate registered snapshots without
    /// downcasting — mirrors `loader_files` above.
    #[returns(ref)]
    pub deployed_schemas: Vec<DeployedSchemaInput>,
}

// ============================================================================
// Deployed-schema snapshot inputs
// ============================================================================

/// A model's previously-deployed schema snapshot, registered as a Salsa
/// world-fact input (`docs/specs/definition_deltas.md` §"Detection": "the
/// deployed-schema snapshot is a world fact both the LSP and the CLI
/// register at workspace load"). One input per `(project_root, model)` pair
/// — project-scoped, per the project-isolation rule, since two projects in
/// the same workspace folder may each maintain a model of the same name.
#[salsa::input]
pub struct DeployedSchemaInput {
    #[returns(ref)]
    pub model: Arc<str>,
    #[returns(ref)]
    pub project_root: PathBuf,
    /// The snapshot's deployed output column names.
    #[returns(ref)]
    pub columns: Vec<Arc<str>>,
    /// The model's source SQL text at the time this schema was deployed —
    /// `None` for a snapshot written before `DeployedSchema::model_sql`
    /// existed. Consulted only by the skeleton-clause-changed check
    /// (`smelt_logical::maintenance::derive::skeleton_clause_changed`).
    #[returns(ref)]
    pub model_sql: Option<Arc<str>>,
    /// The model's declared `timeseries.partition_column` at the time this
    /// schema was deployed — `None` for a snapshot written before
    /// `DeployedSchema::partition_column` existed, or for a model with no
    /// partition grain. Consulted only by the partition-column-rename
    /// refusal check
    /// (`smelt_logical::maintenance::derive::partition_column_changed`).
    #[returns(ref)]
    pub partition_column: Option<Arc<str>>,
}

// ============================================================================
// Loader file inputs
// ============================================================================

/// A per-loader-call file path registered as a Salsa input.
///
/// One `LoaderFileInput` is created per unique loader-target path encountered
/// in the workspace. The LSP (and CLI build orchestration) creates and updates
/// these inputs when the corresponding files change on disk.
#[salsa::input]
pub struct LoaderFileInput {
    /// Workspace-relative path with `/` separators (e.g. `"configs/cohorts.yaml"`).
    #[returns(ref)]
    pub path: Arc<str>,
    /// Raw text of the file. Empty string when the file has not been loaded yet
    /// (callers should always set this before running queries).
    #[returns(ref)]
    pub text: Arc<str>,
    /// Whether the file currently exists in the workspace.
    pub exists: bool,
}

// ============================================================================
// Diagnostics accumulator
// ============================================================================

#[salsa::accumulator]
pub struct DiagnosticAcc(pub Diagnostic);

// ============================================================================
// Database
// ============================================================================

/// The Salsa database for smelt. The `files` and `projects` maps provide a
/// path-keyed registry for looking up input structs — Salsa itself doesn't
/// do this automatically.
#[salsa::db]
#[derive(Clone, Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
    pub(crate) files: Arc<RwLock<HashMap<PathBuf, SourceFile>>>,
    pub(crate) projects: Arc<RwLock<HashMap<PathBuf, ProjectInput>>>,
    /// Per-loader-file inputs keyed by workspace-relative path.
    pub(crate) loader_files: Arc<RwLock<HashMap<String, LoaderFileInput>>>,
    /// Per-deployed-schema inputs keyed by `(project_root, model)`.
    pub(crate) deployed_schemas: Arc<RwLock<HashMap<(PathBuf, String), DeployedSchemaInput>>>,
}

#[salsa::db]
impl salsa::Database for Database {}

/// Read/write a `Database` registry lock, recovering from poisoning instead of
/// panicking. The lock is only poisoned by a panic while the guard is held,
/// which cannot happen in the single-threaded Salsa mutation context these
/// registries are used in; recovering keeps the registry readable rather than
/// cascading a second panic if that invariant is ever violated.
pub(crate) fn read_registry<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn write_registry<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Database {
    /// Create or retrieve the `SourceFile` input for `path`, seeding its fields.
    /// If a `SourceFile` already exists, its text/project_root are updated to
    /// the provided values.
    pub fn set_source_file(
        &mut self,
        path: PathBuf,
        text: String,
        project_root: PathBuf,
    ) -> SourceFile {
        let existing = read_registry(&self.files).get(&path).copied();
        match existing {
            Some(file) => {
                file.set_text(self).to(text);
                file.set_project_root(self).to(project_root);
                file
            }
            None => {
                let file = SourceFile::new(self, path.clone(), text, project_root);
                write_registry(&self.files).insert(path, file);
                file
            }
        }
    }

    /// Create or retrieve the `ProjectInput` for `root`, seeding its yaml.
    ///
    /// Also reads `smelt.yml` from `root` (if present) and stores it in
    /// `smelt_yml_text` so that `project_unstable_schema` is Salsa-tracked
    /// without further call-site changes. The LSP can call
    /// `set_project_smelt_yml` later to propagate in-editor edits.
    pub fn set_project_input(&mut self, root: PathBuf, sources_yaml: String) -> ProjectInput {
        let smelt_yml_text = std::fs::read_to_string(root.join("smelt.yml")).unwrap_or_default();
        let existing = read_registry(&self.projects).get(&root).copied();
        match existing {
            Some(project) => {
                project.set_sources_yaml(self).to(sources_yaml);
                project.set_smelt_yml_text(self).to(smelt_yml_text);
                project
            }
            None => {
                let project = ProjectInput::new(self, root.clone(), sources_yaml, smelt_yml_text);
                write_registry(&self.projects).insert(root, project);
                project
            }
        }
    }

    /// Update the `smelt.yml` text for an already-registered project. Called by
    /// the LSP whenever the file changes on disk; Salsa propagates the
    /// invalidation through `project_unstable_schema` and any query that reads it.
    pub fn set_project_smelt_yml(&mut self, root: &Path, smelt_yml_text: String) {
        let project = read_registry(&self.projects).get(root).copied();
        if let Some(project) = project {
            project.set_smelt_yml_text(self).to(smelt_yml_text);
        }
    }

    /// Look up an already-registered `SourceFile` by path.
    pub fn source_file(&self, path: &Path) -> Option<SourceFile> {
        read_registry(&self.files).get(path).copied()
    }

    /// Look up an already-registered `ProjectInput` by root path.
    pub fn project_input(&self, root: &Path) -> Option<ProjectInput> {
        read_registry(&self.projects).get(root).copied()
    }

    /// Set (or create) the workspace singleton with the given file and project lists.
    ///
    /// Preserves the existing `loader_files` and `active_target` if the workspace
    /// already exists; `set_loader_file` and `set_active_target` manage those fields.
    pub fn set_workspace(&mut self, files: Vec<SourceFile>, projects: Vec<ProjectInput>) {
        match Workspace::try_get(self) {
            Some(ws) => {
                ws.set_files(self).to(files);
                ws.set_projects(self).to(projects);
                // loader_files and active_target are preserved; managed by
                // set_loader_file and set_active_target respectively.
            }
            None => {
                Workspace::new(self, files, projects, Vec::new(), None, Vec::new());
            }
        }
    }

    /// Convenience accessor: the workspace singleton, creating it empty if missing.
    pub fn workspace(&mut self) -> Workspace {
        match Workspace::try_get(self) {
            Some(ws) => ws,
            None => Workspace::new(self, Vec::new(), Vec::new(), Vec::new(), None, Vec::new()),
        }
    }

    /// Set the active build target on the workspace singleton.
    ///
    /// Changing the target causes Salsa to re-evaluate any tracked query that reads
    /// `workspace.active_target(db)`. The loader dispatch reads this value and selects
    /// `loader_resolved_value_with_overlay` when a matching `<basename>.<target>.<ext>`
    /// overlay file exists, falling back to the base file when no overlay is present.
    pub fn set_active_target(&mut self, target: Option<Arc<str>>) {
        if let Some(ws) = Workspace::try_get(self) {
            ws.set_active_target(self).to(target);
        } else {
            Workspace::new(self, Vec::new(), Vec::new(), Vec::new(), target, Vec::new());
        }
    }

    /// Create or update the `LoaderFileInput` for a workspace-relative path.
    ///
    /// Called by the LSP / CLI build orchestration whenever a loader-target file
    /// is discovered (during workspace load) or edited (during an LSP edit
    /// session). Salsa propagates invalidations to `loader_file_parsed` and
    /// `loader_resolved_value` automatically.
    pub fn set_loader_file(
        &mut self,
        path: Arc<str>,
        text: Arc<str>,
        exists: bool,
    ) -> LoaderFileInput {
        let path_str = path.to_string();
        let existing = read_registry(&self.loader_files).get(&path_str).copied();
        match existing {
            Some(input) => {
                input.set_path(self).to(path);
                input.set_text(self).to(text);
                input.set_exists(self).to(exists);
                // The input is already in workspace.loader_files; no structural change needed.
                input
            }
            None => {
                let input = LoaderFileInput::new(self, path, text, exists);
                write_registry(&self.loader_files).insert(path_str, input);
                // Register the new input into the workspace singleton so that
                // Salsa-tracked queries (which receive `&dyn salsa::Database`)
                // can enumerate loader files without downcasting.
                if let Some(ws) = Workspace::try_get(self) {
                    let mut current = ws.loader_files(self).to_vec();
                    current.push(input);
                    ws.set_loader_files(self).to(current);
                }
                input
            }
        }
    }

    /// Look up an already-registered `LoaderFileInput` by workspace-relative path.
    pub fn loader_file(&self, path: &str) -> Option<LoaderFileInput> {
        read_registry(&self.loader_files).get(path).copied()
    }

    /// Create or update the `DeployedSchemaInput` for `(project_root, model)`.
    ///
    /// Called by [`workspace_ingest::register_deployed_schemas_from_disk`]
    /// (CLI `init_db` / LSP `initialize`, workspace-loading-parity rule)
    /// whenever a `.smelt/targets/<target>/schemas/<model>.json` snapshot is
    /// discovered on disk. Salsa propagates invalidations to `maintenance_plan`
    /// / `check_file_diagnostics` automatically — re-setting an already-
    /// registered input's fields (a re-run `register_deployed_schemas_from_disk`
    /// after a fresh deploy) re-invalidates within the same `Database`.
    pub fn set_deployed_schema(
        &mut self,
        model: Arc<str>,
        project_root: PathBuf,
        columns: Vec<Arc<str>>,
        model_sql: Option<Arc<str>>,
        partition_column: Option<Arc<str>>,
    ) -> DeployedSchemaInput {
        let key = (project_root.clone(), model.to_string());
        let existing = read_registry(&self.deployed_schemas).get(&key).copied();
        match existing {
            Some(input) => {
                input.set_columns(self).to(columns);
                input.set_model_sql(self).to(model_sql);
                input.set_partition_column(self).to(partition_column);
                input
            }
            None => {
                let input = DeployedSchemaInput::new(
                    self,
                    model,
                    project_root,
                    columns,
                    model_sql,
                    partition_column,
                );
                write_registry(&self.deployed_schemas).insert(key, input);
                if let Some(ws) = Workspace::try_get(self) {
                    let mut current = ws.deployed_schemas(self).to_vec();
                    current.push(input);
                    ws.set_deployed_schemas(self).to(current);
                }
                input
            }
        }
    }

    /// Look up an already-registered `DeployedSchemaInput` by
    /// `(project_root, model)`.
    pub fn deployed_schema(&self, project_root: &Path, model: &str) -> Option<DeployedSchemaInput> {
        read_registry(&self.deployed_schemas)
            .get(&(project_root.to_path_buf(), model.to_string()))
            .copied()
    }
}
