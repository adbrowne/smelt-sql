//! Eager init-time workspace loading shared between the CLI and the LSP.
//!
//! See `docs/specs/architecture.md` → "Workspace loading parity rule
//! (CLI ↔ LSP)" for the architectural invariant this module encodes. In
//! short: CLI's `init_db` and LSP's `Backend::initialize` BOTH consume
//! [`load_workspace`] so new eager-discovery steps land in one place.
//!
//! What this module handles:
//! - SQL models under `config.paths` (multi-model frontmatter expanded)
//! - Function files under `<project_root>/functions/`
//! - Raw `sources.yml` / `sources.yaml` text (consumed as a Salsa
//!   `ProjectInput` by callers).
//!
//! What this module does NOT handle:
//! - Python model execution (runs Python; lives in the CLI's `python.rs`
//!   and the LSP's `python_scan.rs` for runtime/feature-flag reasons).
//! - Seeds (already shared via the Salsa `project_seeds` query).
//! - Per-entity sources (already shared via the Salsa `project_sources`
//!   query).

use crate::config::{Config, Materialization, ProbesConfig, StateConfig};
use crate::discovery::{ModelDiscovery, ModelFile};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Errors collected while loading a workspace. Mirrors the LSP's
/// `InitErrors` shape so consumers can route messages to the right
/// place (workspace folder, source-file warning, model warning).
#[derive(Debug, Default, Clone)]
pub struct WorkspaceLoadErrors {
    pub workspace_errors: Vec<String>,
    pub source_errors: Vec<String>,
    pub model_errors: Vec<String>,
}

impl WorkspaceLoadErrors {
    pub fn is_empty(&self) -> bool {
        self.workspace_errors.is_empty()
            && self.source_errors.is_empty()
            && self.model_errors.is_empty()
    }

    pub fn total_count(&self) -> usize {
        self.workspace_errors.len() + self.source_errors.len() + self.model_errors.len()
    }

    pub fn merge(&mut self, other: WorkspaceLoadErrors) {
        self.workspace_errors.extend(other.workspace_errors);
        self.source_errors.extend(other.source_errors);
        self.model_errors.extend(other.model_errors);
    }
}

/// Result of loading a single smelt project from disk.
///
/// Callers populate Salsa from this (see `smelt_db::ingest_loaded_workspace`).
#[derive(Debug, Clone)]
pub struct LoadedWorkspace {
    pub project_root: PathBuf,
    /// Project config. Falls back to a minimal default with `paths = ["models"]`
    /// if `smelt.yml` / `smelt.yaml` can't be loaded — the same fallback the
    /// LSP and CLI use today so partial workspaces still light up.
    pub config: Config,
    /// Raw text of `sources.yml` / `sources.yaml`. Empty if absent. Fed into
    /// Salsa via `set_project_input` so per-project source resolution works.
    pub sources_text: String,
    /// Every `.sql` file the workspace exposes as a Salsa `SourceFile`:
    /// models under `config.paths`, plus function declarations under
    /// `functions/`. Multi-model frontmatter is already expanded into
    /// separate `ModelFile` entries with virtual paths.
    pub sql_files: Vec<ModelFile>,
    pub errors: WorkspaceLoadErrors,
}

/// Patch `loaded.sql_files` so open editor buffers override on-disk content
/// on the working-tree side, without introducing a second discovery path
/// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D4). Discovery
/// still happens exactly once, in [`load_workspace`]; this is a post-load
/// patch keyed by path — a path with no matching entry in `loaded.sql_files`
/// (an unsaved new file `load_workspace` never found) is silently ignored,
/// since the workspace-loading-parity rule forbids this function from ever
/// becoming a second loader.
///
/// `overlays` carries only tracked `.sql` model buffers — `smelt.yml` and
/// source YAML are read from disk elsewhere and are deliberately not
/// overlaid (`docs/specs/property_diff.md` §Surface "Editor", Δ2).
pub fn apply_open_buffers(loaded: &mut LoadedWorkspace, overlays: &BTreeMap<PathBuf, String>) {
    if overlays.is_empty() {
        return;
    }
    for model in loaded.sql_files.iter_mut() {
        if let Some(text) = overlays.get(&model.path) {
            model.content = text.clone();
        }
    }
}

/// Load a workspace from `project_root`.
///
/// Pure data — no Salsa interaction here. Read disk, parse, classify, return.
/// `smelt_db::ingest_loaded_workspace` is the canonical sink for this output.
pub fn load_workspace(project_root: &Path) -> LoadedWorkspace {
    let mut errors = WorkspaceLoadErrors::default();

    // Config: same fallback the LSP and CLI use — minimal config with
    // `paths = ["models"]` so partial workspaces still light up.
    let config = Config::load(project_root).unwrap_or_else(|_| Config {
        name: String::new(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: std::collections::HashMap::new(),
        default_materialization: Materialization::View,
        models: std::collections::HashMap::new(),
        python: None,
        target: None,
        state: StateConfig::default(),
        maintenance: None,
        probes: ProbesConfig::default(),
    });

    // Sources: read sources.yml / sources.yaml if present.
    let sources_text = match crate::project::find_config_file(project_root, "sources") {
        Ok(Some(sources_path)) => match std::fs::read_to_string(&sources_path) {
            Ok(content) => content,
            Err(e) => {
                errors.source_errors.push(format!(
                    "Failed to read {}: {}",
                    sources_path.display(),
                    e
                ));
                String::new()
            }
        },
        Ok(None) => String::new(),
        Err(msg) => {
            errors.source_errors.push(msg);
            String::new()
        }
    };

    // SQL files: project-wide walk via ModelDiscovery (D-01 universal
    // discovery). Functions, models, tests, and generators are all returned.
    // `config.paths` is the strip-list for address derivation, not the scan
    // gate.  The `functions/` hardcoded gate is gone — functions anywhere are
    // discovered by classify() inside discover_models().
    let mut sql_files: Vec<ModelFile> = Vec::new();
    let model_discovery = ModelDiscovery::new(project_root.to_path_buf(), config.paths.clone());
    match model_discovery.discover_models() {
        Ok(models) => sql_files.extend(models),
        Err(e) => {
            // discover_models() returns an error only when the project has zero
            // SQL files — a soft warning (empty workspace during onboarding, or
            // a project that only has seeds/sources with no SQL yet). Surface it
            // but keep going.
            errors.model_errors.push(format!("{}", e));
        }
    }
    LoadedWorkspace {
        project_root: project_root.to_path_buf(),
        config,
        sources_text,
        sql_files,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_workspace_returns_empty_for_nonexistent_root() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_workspace(dir.path());
        // No smelt.yml → falls back to paths = ["models"], which doesn't exist
        // → 0 sql_files, but no fatal error. We surface the "no models found"
        // soft warning so the harness can decide what to do.
        assert!(loaded.sql_files.is_empty());
        assert!(loaded.sources_text.is_empty());
    }

    #[test]
    fn load_workspace_finds_models_and_functions() {
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
        assert!(loaded.errors.is_empty(), "got errors: {:?}", loaded.errors);
        assert_eq!(
            loaded.sql_files.len(),
            2,
            "{:?}",
            loaded
                .sql_files
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>()
        );
        let names: Vec<&str> = loaded.sql_files.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"a"), "expected model 'a', got {names:?}");
        assert!(
            names.contains(&"inc"),
            "expected function 'inc', got {names:?}"
        );
    }

    #[test]
    fn load_workspace_excludes_nested_project_files() {
        // A parent project's walk must not claim files belonging to a nested
        // smelt project (one with its own smelt.yml) — see the "Project
        // isolation rule". Without this exclusion, a nested project's file
        // gets registered under both projects, corrupting workspace-wide
        // checks that key on file identity (e.g. duplicate-function-name
        // detection).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: parent\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();

        let nested = dir.path().join("nested");
        std::fs::create_dir_all(nested.join("functions")).unwrap();
        std::fs::write(
            nested.join("smelt.yml"),
            "name: nested\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(
            nested.join("functions").join("inc.sql"),
            "smelt.define inc(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n",
        )
        .unwrap();

        let loaded = load_workspace(dir.path());
        assert!(loaded.errors.is_empty(), "got errors: {:?}", loaded.errors);
        let names: Vec<&str> = loaded.sql_files.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["a"],
            "parent project must not claim the nested project's files: {names:?}"
        );
    }

    #[test]
    fn load_workspace_finds_test_models_in_tests_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::create_dir_all(dir.path().join("tests")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: test_pub\nversion: 1\npaths:\n  - models\n  - tests\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models/simple.sql"), "SELECT 1 AS x\n").unwrap();
        let test_content =
            "smelt.test test_simple AS (SELECT x FROM smelt.simple)\nPASSING simple AS ({x: 1})\nEXPECT ({x: 1})\n";
        std::fs::write(dir.path().join("tests/test_simple.sql"), test_content).unwrap();

        let loaded = load_workspace(dir.path());
        let test_models: Vec<&crate::discovery::ModelFile> =
            loaded.sql_files.iter().filter(|m| m.is_test()).collect();
        assert_eq!(
            test_models.len(),
            1,
            "expected 1 test model; got: {:?}",
            loaded
                .sql_files
                .iter()
                .map(|m| (&m.name, m.is_test()))
                .collect::<Vec<_>>()
        );
        assert_eq!(test_models[0].name, "test_simple");
    }

    #[test]
    fn load_workspace_reads_sources_yml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        let mut sources = std::fs::File::create(dir.path().join("sources.yml")).unwrap();
        sources.write_all(b"sources: []\n").unwrap();
        // Need at least one model so we don't get the empty-workspace soft warning.
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();

        let loaded = load_workspace(dir.path());
        assert!(!loaded.sources_text.is_empty());
        assert!(loaded.sources_text.contains("sources: []"));
    }

    /// S2 (Phase 7 review fix round 1): `apply_open_buffers` is the
    /// post-load patch the property-diff editor integration relies on to
    /// make an unsaved edit visible on the working-tree side
    /// (`docs/specs/property_diff.md` §Surface "Editor", Δ2). It had zero
    /// coverage before this test.
    ///
    /// *Fails against a broken implementation* that matches on the wrong
    /// key (e.g. the model's `name` instead of its `path`, or a
    /// substring/prefix match instead of exact equality): the overlay
    /// would either not apply here (`a.sql`'s content stays
    /// `SELECT 1 AS x`) or would apply to the wrong entry.
    #[test]
    fn apply_open_buffers_overrides_matching_model_path_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();
        std::fs::write(dir.path().join("models").join("b.sql"), "SELECT 2 AS y").unwrap();

        let mut loaded = load_workspace(dir.path());
        assert_eq!(loaded.sql_files.len(), 2);

        let a_path = loaded
            .sql_files
            .iter()
            .find(|m| m.name == "a")
            .expect("model a")
            .path
            .clone();
        let mut overlays = BTreeMap::new();
        overlays.insert(a_path.clone(), "SELECT 999 AS x_edited".to_string());

        apply_open_buffers(&mut loaded, &overlays);

        let a = loaded.sql_files.iter().find(|m| m.name == "a").unwrap();
        let b = loaded.sql_files.iter().find(|m| m.name == "b").unwrap();
        assert_eq!(a.content, "SELECT 999 AS x_edited");
        assert_eq!(
            b.content, "SELECT 2 AS y",
            "a buffer for `a.sql` must not leak onto `b.sql`"
        );
    }

    /// An overlay for a path `load_workspace` never discovered (an
    /// unrelated buffer, or a path typo) is silently ignored rather than
    /// introducing a phantom entry — `apply_open_buffers` is a patch over
    /// discovery, never a second discovery path (D4, workspace-loading
    /// parity). *Fails against a broken implementation* that inserts a new
    /// `ModelFile` for an unmatched overlay: `sql_files.len()` would grow.
    #[test]
    fn apply_open_buffers_ignores_an_overlay_for_an_undiscovered_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();

        let mut loaded = load_workspace(dir.path());
        let before_len = loaded.sql_files.len();

        let mut overlays = BTreeMap::new();
        overlays.insert(
            dir.path().join("models").join("never_discovered.sql"),
            "SELECT 1".to_string(),
        );
        apply_open_buffers(&mut loaded, &overlays);

        assert_eq!(loaded.sql_files.len(), before_len);
        assert_eq!(
            loaded.sql_files[0].content, "SELECT 1 AS x",
            "the real model's content must be untouched"
        );
    }

    /// Δ2 is explicit that `smelt.yml`/source YAML are NOT overlaid — only
    /// tracked `.sql` model buffers are. `apply_open_buffers` only ever
    /// touches `loaded.sql_files`, so an overlay keyed on `smelt.yml`'s own
    /// path (a caller mistake, or a future caller that forgets Δ2) has no
    /// field to land on and is a no-op. *Fails against a broken
    /// implementation* that widens `apply_open_buffers` to also patch
    /// `loaded.sources_text`/config text for a matching path.
    #[test]
    fn apply_open_buffers_does_not_touch_config_or_sources_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("models").join("a.sql"), "SELECT 1 AS x").unwrap();
        let mut sources = std::fs::File::create(dir.path().join("sources.yml")).unwrap();
        sources.write_all(b"sources: []\n").unwrap();

        let mut loaded = load_workspace(dir.path());
        let sources_before = loaded.sources_text.clone();

        let mut overlays = BTreeMap::new();
        overlays.insert(dir.path().join("smelt.yml"), "name: hijacked\n".to_string());
        overlays.insert(
            dir.path().join("sources.yml"),
            "sources: [hijacked]\n".to_string(),
        );
        apply_open_buffers(&mut loaded, &overlays);

        assert_eq!(loaded.sources_text, sources_before);
        assert_eq!(loaded.config.paths, vec!["models".to_string()]);
    }
}
