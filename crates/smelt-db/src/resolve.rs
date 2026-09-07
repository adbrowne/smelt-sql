//! Workspace and project resolution: turning a `smelt.<path>` address (or a
//! leaf name) into the entity it names.
//!
//! Per the Salsa purity rule (`architecture.md` §"Salsa purity rule
//! (analysis)"), the analysis here is pure — `leaf_did_you_mean` takes its
//! inputs as parameters; the Salsa-tracked entries
//! (`project_sql_address_index`, `resolve_source`) are thin wrappers that
//! exist only for incrementality.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use smelt_parser::File as AstFile;

use crate::*;

// ============================================================================
// Semantic queries
// ============================================================================

/// Leaf-only model resolution used by the schema-inference subsystem
/// (`RowExtension.ref_name`, `InputConstraint.ref_name`) and the LSP's
/// column-goto-definition. Architecture Invariant 9 keeps leaf-only
/// resolution out of the value-ref path (`resolve_ref_path` is the
/// canonical path resolver for `smelt.<path>` refs in SQL bodies and
/// CLI argument resolution). The schema layer's column-origin tracking
/// still uses leaf names today; migrating it to canonical paths is a
/// separate refactor (tracked under architecture.md Known Divergences).
///
/// Project-scoped per `docs/specs/architecture.md` → "Project isolation
/// rule": a workspace folder may contain multiple smelt projects, and each
/// project is a closed resolution scope. Without filtering, a same-named
/// model in another project leaks into this project's name lookups.
///
/// Callers thread the project through from the file under analysis:
/// `source_file.project_root(db)` → `find_project(workspace, root)`.
pub fn resolve_ref_leaf(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: ProjectInput,
    model_name: String,
) -> Option<SourceFile> {
    let project_root = project.root(db);
    for file in workspace.files(db).iter().copied() {
        if file.project_root(db) != project_root {
            continue;
        }
        if let Some(model) = parse_model(db, file) {
            if model.name == model_name {
                return Some(file);
            }
        }
    }
    None
}

/// Result of resolving a `smelt.<path>` ref against the workspace.
///
/// Phase 2a unifies model / seed / source / function / test resolution
/// behind a single entry point — [`resolve_ref_path`]. Callers dispatch
/// on `kind` to decide what to do; `source_file` is populated for
/// `Model`, `Function`, and `Test` kinds (the entity lives in a
/// `.sql` file tracked by Salsa).
#[derive(Clone)]
pub struct ResolvedRef {
    pub kind: RefKind,
    /// The Salsa-tracked file backing the entity. Populated for
    /// `Model` / `Function` / `Test`. `None` for seeds and sources
    /// (which live outside the SQL file index).
    pub source_file: Option<SourceFile>,
    /// The path tuple used to perform the lookup, for round-tripping
    /// into diagnostics.
    pub path: Vec<String>,
}

impl std::fmt::Debug for ResolvedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRef")
            .field("kind", &self.kind)
            .field("source_file", &self.source_file.is_some())
            .field("path", &self.path)
            .finish()
    }
}

/// Resolve a path tuple (`["models", "users"]`,
/// `["seeds", "raw", "users"]`, …) against the workspace.
///
/// Per architecture Surface §"Resolution: smelt.<path> is the universal
/// addressing scheme":
/// - `.sql` file with a bare SELECT → `Model`
/// - `.sql` file declaring `smelt.define` → `Function`
/// - `.sql` file containing `smelt.test` declarations → `Test`
/// - `.csv` under a project's `paths` → `Seed`
/// - `.yml` declaring an external table → `Source`
///
/// The tuple is matched against each workspace `SourceFile`'s path,
/// falling back to seed/source registries for non-SQL kinds. Kind
/// dispatch is by file format/content, never by directory name.
pub fn resolve_ref_path(
    db: &dyn salsa::Database,
    workspace: Workspace,
    path: Vec<String>,
) -> Option<ResolvedRef> {
    if path.is_empty() {
        return None;
    }

    // Try every project root in the workspace; the first match wins.
    for project in workspace.projects(db).iter().copied() {
        let project_root = project.root(db).clone();
        // Fetch the project's scan-root list once (cached via Salsa) and pass
        // it into `file_path_tuple` for every workspace file. Without this
        // hoist, each iteration of the file loop below would re-parse
        // `smelt.yml` from disk inside `file_path_tuple`, which scaled the
        // resolver to O(workspace_files * config_load_cost) per call.
        let scan_roots = project_paths(db, project);

        // Seeds: match by address_segments (Phase 2 — no "seeds" prefix required).
        // address_segments is the scan-root-stripped path tuple, so
        // `smelt.data.users` matches a seed at `seeds/data/users.csv` under
        // `paths: ["seeds"]` with address_segments = ["data", "users"].
        for seed in project_seeds(db, project).iter() {
            if seed.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Seed,
                    source_file: None,
                    path,
                });
            }
        }

        // Sources: Phase 6 per-entity YAML files. Each source has an
        // `address_segments` tuple (scan-root-stripped path to stem).
        // `smelt.sources.raw.users` → path = ["sources", "raw", "users"]
        // which matches the `.yml` at `models/sources/raw/users.yml`.
        for source in project_sources(db, project).iter() {
            if source.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // Legacy sources: project-level aggregate `sources.yml`. Used as a
        // fallback for any projects not yet migrated to per-entity YAMLs.
        // Kept until Phase 6 migration is complete across all callers.
        if project_sources(db, project).is_empty() && path.len() >= 3 && path[0] == "sources" {
            let source_name = &path[path.len() - 2];
            let table_name = &path[path.len() - 1];
            if resolve_source(db, project, source_name.clone(), table_name.clone()).is_some() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // SQL files: O(1) lookup in the per-project address index instead of
        // rescanning every workspace file and recomputing its path tuple.
        // The index (`project_sql_address_index`) is a workspace-keyed tracked
        // query, so the scan runs once per revision rather than once per ref —
        // collapsing cold ref resolution from O(files × refs) to O(refs).
        if let Some((kind, file)) = project_sql_address_index(db, workspace, project).get(&path) {
            return Some(ResolvedRef {
                kind: *kind,
                source_file: Some(*file),
                path,
            });
        }

        // Generator-emitted models: check the W3 emission survivors for a path
        // match. Emitted models are not registered as SourceFile inputs, so they
        // are not found in the SQL-files walk above. The smelt path of an emitted
        // model is `<dir_dots>.<file_stem>.<ModelDef.name>` (from
        // `emitted_model_smelt_path`), and the dot-separated components equal the
        // `path` Vec we are resolving.
        let emitted = crate::queries::project::emitted_models(db, workspace);
        for emitted_model in &emitted.survivors {
            if !emitted_model.generator_file.starts_with(&project_root) {
                continue;
            }
            let smelt_name = crate::queries::project::emitted_model_smelt_path(
                &emitted_model.generator_file,
                &project_root,
                scan_roots.as_slice(),
                &emitted_model.name,
            );
            let emitted_path: Vec<String> = smelt_name.split('.').map(|s| s.to_string()).collect();
            if emitted_path == path {
                // Return a ResolvedRef pointing at the generator file; the
                // goto-def handler will navigate to the ModelDef.name span within it.
                // Look up the generator file's SourceFile handle from workspace files.
                let gen_file = workspace
                    .files(db)
                    .iter()
                    .copied()
                    .find(|f| f.path(db) == &emitted_model.generator_file);
                return Some(ResolvedRef {
                    kind: RefKind::Model,
                    source_file: gen_file,
                    path,
                });
            }
        }
    }

    None
}

/// Compute the path tuple for a SQL file relative to its project root,
/// stripping the matching `config.paths` scan-root prefix if one applies.
///
/// Algorithm:
/// 1. Strip `project_root` to get `rel`.
/// 2. Try each scan root from `config.paths`: if `rel` starts with the
///    scan root, use the remainder as the parent path.
/// 3. If no scan root matches (e.g. `functions/` with `paths: ["models"]`),
///    fall back to using `rel.parent()` (original behaviour).
/// 4. Build tuple from parent segments + leaf name (model name override as today).
///
/// Returns `None` if the file is not a descendant of the project root.
fn file_path_tuple(
    project_root: &Path,
    file_path: &Path,
    file: SourceFile,
    db: &dyn salsa::Database,
    scan_roots: &[String],
) -> Option<Vec<String>> {
    let rel = file_path.strip_prefix(project_root).ok()?;

    // Try each scan root. Use the first one that `rel` is under.
    let effective_rel = scan_roots
        .iter()
        .find_map(|sr| rel.strip_prefix(sr.as_str()).ok())
        .unwrap_or(rel);

    let parent = effective_rel.parent()?;
    let mut tuple: Vec<String> = parent
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // Leaf segment: prefer the parsed model name (so multi-model files
    // expose their declared `name:` rather than the filename), falling
    // back to the file stem for non-model SQL files (functions, tests).
    let leaf = parse_model(db, file).map(|m| m.name.clone()).or_else(|| {
        file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })?;
    tuple.push(leaf);
    Some(tuple)
}

/// Determine the kind of a SQL file by its content. Model, function,
/// and test all live in `.sql` files; the dispatch is on
/// content/frontmatter, not filename.
fn sql_file_kind(db: &dyn salsa::Database, file: SourceFile) -> RefKind {
    // 1. `smelt.define` → Function; `smelt.test` → Test. Both dispatch on
    //    the parsed AST (already cached by Salsa).
    let parse = parse_file(db, file);
    if let Some(ast) = AstFile::cast(parse.syntax()) {
        if ast.defines().next().is_some() {
            return RefKind::Function;
        }
        if ast.tests().next().is_some() {
            return RefKind::Test;
        }
        if ast.checks().next().is_some() {
            return RefKind::Check;
        }
    }
    // 2. Default: Model.
    RefKind::Model
}

/// One-pass index from a project's SQL-file path tuples to their
/// `(RefKind, SourceFile)`, keyed on the [`Workspace`] + [`ProjectInput`].
///
/// `resolve_ref_path` previously rescanned **every** workspace file (computing
/// `file_path_tuple` for each) on every call, making a cold diagnostics pass
/// O(files × refs × files) — the dominant `std::path` cost in the Initial Load
/// benchmark. Hoisting that scan into one workspace-keyed query collapses the
/// per-ref cost to an O(1) `HashMap` lookup; the scan runs once per revision and
/// is shared by every resolver call. This mirrors `workspace_function_signatures`.
///
/// First-writer-wins on tuple collisions, preserving the original loop's
/// "first matching file in `workspace.files` order wins" semantics.
#[salsa::tracked]
pub fn project_sql_address_index(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: ProjectInput,
) -> Arc<HashMap<Vec<String>, (RefKind, SourceFile)>> {
    let project_root = project.root(db).clone();
    let scan_roots = project_paths(db, project);
    let mut map: HashMap<Vec<String>, (RefKind, SourceFile)> = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        let file_path = file.path(db);
        // Mirror the resolver's file filter: SQL models, Python models (whose
        // content is generated SQL), virtual `*.sql::model` split paths, and
        // virtual `*.py::name` paths for Python-emitted models.
        // Note: Path::extension() on "py_source.py::py_source" returns
        // "py::py_source" (everything after the last dot), not "py", so the
        // .py:: check is required to catch Python virtual paths.
        let path_str = file_path.to_str().unwrap_or("");
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "sql"
            && ext != "py"
            && !path_str.contains(".sql::")
            && !path_str.contains(".py::")
        {
            continue;
        }
        let Some(tuple) = file_path_tuple(&project_root, file_path, file, db, &scan_roots) else {
            continue;
        };
        map.entry(tuple)
            .or_insert_with(|| (sql_file_kind(db, file), file));
    }
    Arc::new(map)
}

/// Find every canonical `smelt.<path>` address in `workspace` whose leaf
/// segment equals `leaf`, scoped to `project` per the Project Isolation Rule.
///
/// Returns the canonical paths (with the `smelt.` prefix) sorted
/// alphabetically. The result is used by the `UndefinedModelRef` diagnostic to
/// generate a "did you mean …?" hint.
///
/// This is a **pure function**: it receives all necessary data as parameters
/// and performs no Salsa query calls internally — Salsa queries are called by
/// the callers that gather the inputs before passing them in.
///
/// # Project Isolation Rule
/// When `project` is `Some`, only files belonging to that project are
/// considered. Two projects in the same workspace folder do not share leaf-match
/// candidates.
pub fn leaf_did_you_mean(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    leaf: &str,
) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();

    for file in workspace.files(db).iter().copied() {
        // Project isolation: skip files from other projects.
        if let Some(p) = project {
            if file.project_root(db) != p.root(db) {
                continue;
            }
        }

        // Only SQL or Python-emitted model files can be models.
        let file_path = file.path(db);
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let path_str = file_path.to_str().unwrap_or("");
        if ext != "sql"
            && ext != "py"
            && !path_str.contains(".sql::")
            && !path_str.contains(".py::")
        {
            continue;
        }

        // Get the leaf segment: parse_model gives us the declared model name;
        // fall back to file stem for non-model files.
        let file_leaf = parse_model(db, file).map(|m| m.name.clone()).or_else(|| {
            file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });

        if file_leaf.as_deref() != Some(leaf) {
            continue;
        }

        // Compute the canonical path for this file using the project's scan roots.
        // We need to determine which project this file belongs to in order to
        // get the correct scan roots.
        let project_for_file = project.or_else(|| {
            let file_root = file.project_root(db).clone();
            workspace
                .projects(db)
                .iter()
                .copied()
                .find(|p| p.root(db) == &file_root)
        });

        let tuple = if let Some(p) = project_for_file {
            let scan_roots = project_paths(db, p);
            file_path_tuple(p.root(db), file_path, file, db, &scan_roots)
        } else {
            None
        };

        if let Some(t) = tuple {
            candidates.push(format!("smelt.{}", t.join(".")));
        }
    }

    // Sort alphabetically for deterministic output.
    candidates.sort();
    candidates
}

#[salsa::tracked]
pub fn resolve_source(
    db: &dyn salsa::Database,
    project: ProjectInput,
    source_name: String,
    table_name: String,
) -> Option<SourceTableDef> {
    let config = sources_config(db, project);
    let source = config.sources.iter().find(|s| s.name == source_name)?;
    source.tables.iter().find(|t| t.name == table_name).cloned()
}

/// Resolve a project root path to a `ProjectInput` via the workspace.
///
/// Public so the LSP can derive the caller's project (from the cursor
/// file's `project_root(db)`) when threading the project isolation rule
/// through goto-def, hover, and other features that consult function
/// signatures. See `docs/specs/architecture.md` → "Project isolation rule".
pub fn find_project(
    db: &dyn salsa::Database,
    workspace: Workspace,
    root: &Path,
) -> Option<ProjectInput> {
    workspace
        .projects(db)
        .iter()
        .copied()
        .find(|p| p.root(db) == root)
}

/// Look up the registered [`DeployedSchemaInput`] for `(project_root, table)`
/// via the `Workspace` singleton's `deployed_schemas` list — the enumeration
/// seam a Salsa-tracked query (`&dyn salsa::Database`, no downcast to the
/// concrete `Database`) must use, mirroring `workspace.loader_files(db)`'s
/// lookup pattern in `queries/loader.rs`/`queries/project.rs`.
pub(crate) fn find_deployed_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project_root: &Path,
    table: &str,
) -> Option<DeployedSchemaInput> {
    workspace
        .deployed_schemas(db)
        .iter()
        .copied()
        .find(|s| s.project_root(db) == project_root && &**s.model(db) == table)
}
