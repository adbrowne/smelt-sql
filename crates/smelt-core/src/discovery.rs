use crate::config::Materialization;
use crate::metadata::{extract_file_metadata, FileMetadata, ModelMetadata, TestConfig};
use crate::model_id::ModelId;
use crate::refs::extract_refs;
pub use crate::refs::RefInfo;
use anyhow::{anyhow, Context, Result};
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use tracing::warn;
use walkdir::WalkDir;

/// A query recorded during Python model execution.
///
/// Pure data — lives outside the feature-gated `python_models` module so the
/// CLI's discovery can attach it to `ModelFile::kind = ModelKind::Python {..}`
/// without depending on PyO3. `Deserialize` is derived so the CLI's
/// subprocess runner can decode JSON output from `smelt.runner` straight
/// into this shape (matches the JSON shape: `{kind, tag, directory}`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PythonModelQuery {
    pub kind: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
}

/// Whether a model comes from a SQL file or Python generation.
#[derive(Debug, Clone)]
pub enum ModelKind {
    Sql,
    Python {
        source_line: usize,
        queries: Vec<PythonModelQuery>,
    },
}

#[derive(Debug, Clone)]
pub struct ModelFile {
    pub name: String,
    /// Path used as the Salsa query key (virtual for multi-model files).
    pub path: PathBuf,
    pub content: String,
    pub refs: Vec<RefInfo>,
    pub parse_errors: Vec<smelt_parser::ParseError>,
    /// Metadata extracted from YAML frontmatter
    pub metadata: Option<Box<ModelMetadata>>,
    /// Whether this model is from a SQL file or Python generation.
    pub kind: ModelKind,
    /// Canonical model identifier.
    pub model_id: ModelId,
    /// Address segments with scan-root prefix stripped, e.g. `["staging", "stg_events"]`
    /// for `models/staging/stg_events.sql` under `paths: ["models"]`.
    /// Empty if the scan root couldn't be determined.
    /// The default DB table name is `address_segments.join("_")`.
    pub address_segments: Vec<String>,
}

impl ModelFile {
    /// Canonical dot-joined `smelt.<path>` address of this model.
    /// Equals `self.address_segments.join(".")`.
    pub fn canonical_path(&self) -> String {
        self.address_segments.join(".")
    }

    /// Whether this model is a test.
    pub fn is_test(&self) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.materialization.as_ref())
            .map(|m| *m == Materialization::Test)
            .unwrap_or(false)
    }

    /// Get test configuration if this is a test model.
    pub fn test_config(&self) -> Option<&TestConfig> {
        self.metadata.as_ref().and_then(|m| m.test.as_ref())
    }

    /// The default database table name for this model: address segments joined
    /// with `_`. Falls back to `self.name` if `address_segments` is empty.
    ///
    /// Examples:
    ///   - `["staging", "stg_events"]` → `"staging_stg_events"`
    ///   - `["base"]` → `"base"`
    ///   - `[]` (fallback) → `self.name`
    pub fn db_name(&self) -> &str {
        // address_segments is lazily computed; fall back to name if absent.
        // We can't join here (returns String not &str), so callers that need
        // a String should call `self.address_segments.join("_")` directly
        // or use `db_name_owned()`.
        if self.address_segments.is_empty() {
            &self.name
        } else {
            // Return the last segment as a fallback for &str API.
            // Callers needing the full joined name must call db_name_owned().
            self.address_segments
                .last()
                .map(|s| s.as_str())
                .unwrap_or(&self.name)
        }
    }

    /// The default database table name as an owned String.
    pub fn db_name_owned(&self) -> String {
        if self.address_segments.is_empty() {
            self.name.clone()
        } else {
            self.address_segments.join("_")
        }
    }
}

/// Walk every non-excluded file under `project_root`, grouped by directory.
///
/// Excluded (fixed skip-list, per spec architecture.md §"Resolution"):
/// - Any directory whose name starts with `.` (`.git`, `.smelt`, etc.)
/// - Any directory named `target` (Cargo / smelt build output)
///
/// Returns a list of `(dir, files_in_dir)` pairs where `files_in_dir` contains
/// all files found directly inside `dir`. The sibling list is used by callers
/// that need `classify()` for sidecar detection.
pub fn project_root_files_by_dir(project_root: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut by_dir: std::collections::HashMap<PathBuf, Vec<PathBuf>> =
        std::collections::HashMap::new();

    for entry in WalkDir::new(project_root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            // Never prune the root itself (depth 0 — the project root may be in
            // a path component that starts with `.`, e.g. a hidden parent dir).
            // Only prune *subdirectories* whose names match the exclusion list.
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                let name = e.file_name().to_str().unwrap_or("");
                !name.starts_with('.') && name != "target"
            } else {
                true
            }
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        let dir = path
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .to_path_buf();
        by_dir.entry(dir).or_default().push(path);
    }

    by_dir.into_iter().collect()
}

/// Discover `smelt.define` / `smelt.extern` files under the project root.
///
/// Returns the paths of every `.sql` file under any non-excluded directory that
/// contains a `smelt.define` (or `smelt.extern`) declaration. Discovery is
/// project-wide — no dedicated `functions/` gate; functions live wherever they
/// are declared (D-01/D-05). Hidden directories and `target/` are excluded.
///
/// Both the CLI's `Discovery::discover_function_files` and the LSP's
/// `initialize` call this to keep their function-discovery behavior in sync.
pub fn discover_function_file_paths(project_root: &Path) -> Vec<PathBuf> {
    use crate::resolver::{classify, EntityKind};

    let mut paths = Vec::new();
    for (_, files) in project_root_files_by_dir(project_root) {
        for file_path in &files {
            if matches!(
                classify(file_path, None, &files),
                Some(EntityKind::Function)
            ) {
                paths.push(file_path.clone());
            }
        }
    }
    paths.sort(); // deterministic ordering
    paths
}

/// Strip the first matching `paths:` prefix from a project-root-relative path.
///
/// This implements the D-01 addressing rule: `paths:` is a strip-list, not a scan
/// gate. For a path relative to the project root (e.g. `models/staging/x.sql`) and
/// `paths: ["models"]`, the result is `staging/x.sql`. If no prefix matches, the
/// path is returned unchanged (preserving the full project-relative path).
pub fn strip_paths_prefix<'a>(rel: &'a Path, paths: &[String]) -> &'a Path {
    for prefix in paths {
        if let Ok(stripped) = rel.strip_prefix(prefix.as_str()) {
            return stripped;
        }
    }
    rel
}

pub struct ModelDiscovery {
    project_root: PathBuf,
    paths: Vec<String>,
}

impl ModelDiscovery {
    pub fn new(project_root: PathBuf, paths: Vec<String>) -> Self {
        Self {
            project_root,
            paths,
        }
    }

    pub fn discover_models(&self) -> Result<Vec<ModelFile>> {
        use crate::resolver::{classify, EntityKind};

        let mut models = Vec::new();

        // Universal walk: visit every non-excluded file under the project root.
        // `paths:` is the strip-list (for address derivation), NOT the scan gate.
        for (_, files) in project_root_files_by_dir(&self.project_root) {
            for file_path in &files {
                // Classify by content/extension (sidecar-aware via siblings list).
                match classify(file_path, None, &files) {
                    Some(EntityKind::Model)
                    | Some(EntityKind::Function)
                    | Some(EntityKind::Test) => {
                        // Parse (address_segments filled in below).
                        let mut parsed = match self.parse_model_file(file_path) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("Failed to parse {}: {}", file_path.display(), e);
                                continue;
                            }
                        };
                        // Address via the D-01 strip-list rule.
                        let address_segments = Self::compute_address_segments(
                            file_path,
                            &self.project_root,
                            &self.paths,
                        );
                        for m in &mut parsed {
                            m.address_segments = address_segments.clone();
                            // For multi-model files, replace the leaf with the
                            // declared model name.
                            if let Some(last) = m.address_segments.last_mut() {
                                *last = m.name.clone();
                            }
                        }
                        models.extend(parsed);
                    }
                    // Seeds (CSV), sources (standalone YAML), and sidecars
                    // are handled by their own discovery paths — skip here.
                    // Generator files classify as Model (no smelt.define/test);
                    // parse_sql_file handles the Generator frontmatter arm.
                    _ => {}
                }
            }
        }

        if models.is_empty() {
            return Err(anyhow!("No SQL files found in project"));
        }

        Ok(models)
    }

    /// Compute address segments for a file at `path` using the D-01 strip-list rule.
    ///
    /// Algorithm:
    /// 1. Strip `project_root` from `path` to get the project-relative path.
    /// 2. Strip the first matching `paths:` prefix (`strip_paths_prefix`).
    /// 3. Return parent-directory components + file stem as address segments.
    ///
    /// Examples under `paths: ["models"]`:
    ///   `models/staging/stg_events.sql` → `["staging", "stg_events"]`
    ///   `functions/helper.sql`          → `["functions", "helper"]`  (no match → full rel path)
    ///   `foo.sql` (project root)        → `["foo"]`  (bare name)
    ///
    /// Pass `paths = &[]` for callers that want the full project-relative path with
    /// no prefix stripping (e.g. `parse_sql_file` for function files).
    pub fn compute_address_segments(
        path: &Path,
        project_root: &Path,
        paths: &[String],
    ) -> Vec<String> {
        let Ok(rel) = path.strip_prefix(project_root) else {
            return Vec::new();
        };
        let stripped = strip_paths_prefix(rel, paths);
        let parent = stripped.parent().unwrap_or(std::path::Path::new(""));
        let mut segs: Vec<String> = parent
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        // Leaf: file stem (will be replaced with model name for multi-model files).
        if let Some(stem) = stripped
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        {
            segs.push(stem);
        }
        segs
    }

    fn parse_model_file(&self, path: &Path) -> Result<Vec<ModelFile>> {
        // `discover_models` fills in `address_segments` after this call, so
        // pass `None` for scan_root here (the parent loop handles it).
        parse_sql_file(path, None)
    }
}

/// Parse a `.sql` file into one or more `ModelFile`s.
///
/// Pure function — reads the file, expands multi-model frontmatter, parses
/// each section for refs, and returns the resulting `ModelFile` entries.
///
/// When `scan_root` is `Some`, `address_segments` is populated using
/// [`ModelDiscovery::compute_address_segments`]: the path relative to
/// `scan_root`, with directory components as prefix and the model name (or
/// file stem for single-model files) as the leaf segment. The caller should
/// pass `project_root` as the scan root for files discovered outside
/// `config.paths` (e.g. `functions/`) so they keep their full
/// workspace-relative path.
///
/// When `scan_root` is `None`, `address_segments` is left empty and the
/// caller is responsible for computing it (e.g. `ModelDiscovery::discover_models`
/// does this in a post-pass).
///
/// Shared by `ModelDiscovery::discover_models` and by
/// `smelt_core::workspace::load_workspace` so multi-model handling is in one
/// place; the LSP's `register_sql_content` is the third (LSP-specific) copy
/// that should eventually delegate here too.
pub fn parse_sql_file(path: &Path, scan_root: Option<&Path>) -> Result<Vec<ModelFile>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read model file: {:?}", path))?;

    // Extract metadata from YAML frontmatter
    let file_metadata = match extract_file_metadata(&content) {
        Ok(fm) => Some(fm),
        Err(e) => {
            warn!("frontmatter parse error in {}: {}", path.display(), e);
            None
        }
    };

    match file_metadata {
        Some(FileMetadata::Generator { metadata, .. }) => {
            // Generator files (`generates: models` frontmatter) are handled by the
            // W1–W4 Salsa pipeline, not by the SQL model discovery path.  Return
            // the file's content so the Salsa DB can register it (for the
            // `generator_files` query), but skip SQL parsing — the body is a
            // meta-language expression, not a `smelt.define` or SELECT statement.
            //
            // Spec: meta_language.md §"Multi-model production" — "The `.gen.sql`
            // extension is a recommended convention; it is **not load-bearing**.
            // The compiler determines a file's status from the frontmatter alone."
            // (BUG-066 fix: previously the `_ =>` arm parsed generators as SQL.)
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Cannot determine model name from {:?}", path))?;

            let address_segments = scan_root
                .map(|root| ModelDiscovery::compute_address_segments(path, root, &[]))
                .unwrap_or_default();

            Ok(vec![ModelFile {
                name,
                path: path.to_path_buf(),
                content,
                refs: vec![],
                parse_errors: vec![],
                metadata: Some(metadata),
                kind: ModelKind::Sql,
                model_id: ModelId::from_path(path.to_path_buf()),
                address_segments,
            }])
        }
        Some(FileMetadata::Multi { models }) => {
            // Multi-model file: create one ModelFile per section
            let base_segments = scan_root
                .map(|root| ModelDiscovery::compute_address_segments(path, root, &[]))
                .unwrap_or_default();

            let mut result = Vec::with_capacity(models.len());
            for section in models {
                let model_name = section
                    .metadata
                    .name
                    .clone()
                    .ok_or_else(|| anyhow!("Multi-model section missing name in {:?}", path))?;

                let sql_content = &content[section.sql_range.clone()];
                let clean_content = smelt_parser::strip_frontmatter(sql_content);
                let parse = smelt_parser::parse(&clean_content);
                let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
                    extract_refs(&file)
                } else {
                    Vec::new()
                };

                let model_id = ModelId::multi_model(path.to_path_buf(), model_name.clone());

                // Replace the leaf segment with the declared model name.
                let mut address_segments = base_segments.clone();
                if let Some(last) = address_segments.last_mut() {
                    *last = model_name.clone();
                }

                result.push(ModelFile {
                    name: model_name,
                    path: model_id.salsa_key(),
                    content: sql_content.to_string(),
                    refs,
                    parse_errors: parse.errors,
                    metadata: Some(Box::new(section.metadata)),
                    kind: ModelKind::Sql,
                    model_id,
                    address_segments,
                });
            }
            Ok(result)
        }
        _ => {
            // Single-model or no frontmatter: existing behavior
            let model_metadata = match file_metadata {
                Some(FileMetadata::Single { metadata, .. }) => Some(metadata),
                _ => None,
            };

            // Spec `models.md` §"Model naming": for single-model files the file
            // stem is *always* authoritative. The `name:` frontmatter key is
            // accepted (so YAML round-trips) but has no effect on identity. This
            // matches `smelt-db`'s `parse_model` (which keys on `file_stem`);
            // honouring `name:` here would create an LSP↔CLI naming asymmetry.
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Cannot determine model name from {:?}", path))?;

            // Strip frontmatter before parsing so the parser sees only SQL
            // (matches the CLI's existing behavior; single source of truth).
            let clean_content = smelt_parser::strip_frontmatter(&content);
            let parse = smelt_parser::parse(&clean_content);
            let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
                extract_refs(&file)
            } else {
                Vec::new()
            };

            let model_id = ModelId::from_path(path.to_path_buf());

            // Compute address_segments if scan_root was provided.
            // For single-model files, the leaf segment is the model name
            // (which may differ from the file stem if frontmatter declares
            // a `name:` override). Replace the file-stem leaf with the name.
            let address_segments = if let Some(root) = scan_root {
                let mut segs = ModelDiscovery::compute_address_segments(path, root, &[]);
                if let Some(last) = segs.last_mut() {
                    *last = name.clone();
                }
                segs
            } else {
                Vec::new()
            };

            Ok(vec![ModelFile {
                name,
                path: path.to_path_buf(),
                content,
                refs,
                parse_errors: parse.errors,
                metadata: model_metadata,
                kind: ModelKind::Sql,
                model_id,
                address_segments,
            }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_refs() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.models.raw_events
GROUP BY user_id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].smelt_ref.to_path().join("."), "models.raw_events");
        assert!(!refs[0].has_named_params);
    }

    #[test]
    fn test_extract_refs_with_named_params() {
        // Named params come from SmeltPathCall nodes (path-form function calls).
        let sql = r#"
SELECT smelt.functions.format_date(d => event_date, fmt => 'YYYY-MM-DD') AS formatted
FROM raw_events
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].smelt_ref.leaf_name(), "format_date");
        assert!(refs[0].has_named_params);
    }

    #[test]
    fn test_multi_model_file_discovery() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let multi_model_content = r#"--- name: staging_events ---
materialization: view
---
SELECT * FROM raw_events

--- name: cleaned_events ---
materialization: table
---
SELECT * FROM smelt.models.staging_events
"#;

        let file_path = models_dir.join("pipeline.sql");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(multi_model_content.as_bytes()).unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();

        assert_eq!(
            models.len(),
            2,
            "Should discover 2 models from multi-model file"
        );

        let staging = models.iter().find(|m| m.name == "staging_events").unwrap();
        assert!(staging.model_id.is_multi_model);
        assert!(staging.content.contains("SELECT * FROM raw_events"));
        assert!(!staging.content.contains("cleaned_events"));

        let cleaned = models.iter().find(|m| m.name == "cleaned_events").unwrap();
        assert!(cleaned.model_id.is_multi_model);
        assert!(cleaned.content.contains("smelt.models.staging_events"));
        assert_eq!(cleaned.refs.len(), 1);
        assert_eq!(
            cleaned.refs[0].smelt_ref.to_path().join("."),
            "models.staging_events"
        );

        // Virtual paths should be different
        assert_ne!(staging.path, cleaned.path);
        // But source paths should be the same
        assert_eq!(
            staging.model_id.source_path(),
            cleaned.model_id.source_path()
        );
    }

    #[test]
    fn test_discover_function_file_paths_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let paths = discover_function_file_paths(dir.path());
        assert!(paths.is_empty());
    }

    #[test]
    fn test_discover_function_file_paths_finds_sql_files() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let functions_dir = dir.path().join("functions");
        let nested_dir = functions_dir.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();

        let top = functions_dir.join("sessionize.sql");
        std::fs::File::create(&top)
            .unwrap()
            .write_all(b"smelt.define sessionize(x: Expr<Integer>) AS (x)")
            .unwrap();
        let nested = nested_dir.join("inner.sql");
        std::fs::File::create(&nested)
            .unwrap()
            .write_all(b"smelt.define inner(x: Expr<Integer>) AS (x + 1)")
            .unwrap();
        // Non-sql files are ignored.
        let other = functions_dir.join("README.md");
        std::fs::File::create(&other)
            .unwrap()
            .write_all(b"docs")
            .unwrap();

        let paths = discover_function_file_paths(dir.path());
        assert_eq!(paths.len(), 2, "expected 2 .sql files, got {paths:?}");
        assert!(paths.contains(&top));
        assert!(paths.contains(&nested));
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT
    a.user_id,
    b.session_id
FROM smelt.models.model_a a
INNER JOIN smelt.models.model_b b ON a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].smelt_ref.to_path().join("."), "models.model_a");
        assert_eq!(refs[1].smelt_ref.to_path().join("."), "models.model_b");
    }

    // ----- canonical_path() tests (Phase 1) --------------------------------

    /// A model at `models/silver/events_parsed.sql` under `paths: ["models"]`
    /// must yield canonical_path() == "silver.events_parsed".
    #[test]
    fn canonical_path_single_model_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let silver_dir = dir.path().join("models").join("silver");
        std::fs::create_dir_all(&silver_dir).unwrap();

        let file_path = silver_dir.join("events_parsed.sql");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"SELECT 1 AS x")
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].canonical_path(), "silver.events_parsed");
    }

    /// A multi-model file at `models/staging/pairs.sql` declaring two models
    /// must yield canonical paths "staging.orders" and "staging.customers".
    #[test]
    fn canonical_path_multi_model_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let staging_dir = dir.path().join("models").join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        let content = r#"--- name: orders ---
materialization: view
---
SELECT 1 AS id

--- name: customers ---
materialization: view
---
SELECT 2 AS id
"#;
        std::fs::File::create(staging_dir.join("pairs.sql"))
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        assert_eq!(models.len(), 2);

        let orders = models.iter().find(|m| m.name == "orders").unwrap();
        assert_eq!(orders.canonical_path(), "staging.orders");

        let customers = models.iter().find(|m| m.name == "customers").unwrap();
        assert_eq!(customers.canonical_path(), "staging.customers");
    }

    /// A model file discovered outside the project's `paths:` (e.g. under
    /// `functions/`) keeps the full workspace-relative path joined by `.`.
    /// For `functions/patterns/sessionize.sql` with `project_root` as scan
    /// root, canonical_path() == "functions.patterns.sessionize".
    #[test]
    fn canonical_path_no_scan_root_match() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let patterns_dir = dir.path().join("functions").join("patterns");
        std::fs::create_dir_all(&patterns_dir).unwrap();

        std::fs::File::create(patterns_dir.join("sessionize.sql"))
            .unwrap()
            .write_all(b"smelt.define sessionize() AS (SELECT 1)")
            .unwrap();

        // Simulate the segments that load_workspace computes for function
        // files: project_root as root, no paths stripping → full workspace-relative path.
        let file_path = patterns_dir.join("sessionize.sql");
        let segs = ModelDiscovery::compute_address_segments(&file_path, dir.path(), &[]);
        assert_eq!(segs, vec!["functions", "patterns", "sessionize"]);

        let mut model = ModelFile {
            name: "sessionize".to_string(),
            path: file_path.clone(),
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: crate::model_id::ModelId::from_path(file_path),
            address_segments: segs,
        };
        // For a single-function file the leaf segment is the file stem which
        // matches the model name, so no replacement is needed. But for a
        // multi-model function file the caller would replace the leaf.
        // canonical_path() always just joins whatever segments are present.
        assert_eq!(model.canonical_path(), "functions.patterns.sessionize");

        // After leaf-replacement with model name (multi-function case):
        if let Some(last) = model.address_segments.last_mut() {
            *last = "sessionize".to_string();
        }
        assert_eq!(model.canonical_path(), "functions.patterns.sessionize");
    }

    /// A model directly under the scan root (`models/users.sql` with
    /// `paths: ["models"]`) yields canonical_path() == "users".
    #[test]
    fn canonical_path_at_scan_root() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        std::fs::File::create(models_dir.join("users.sql"))
            .unwrap()
            .write_all(b"SELECT 1 AS id")
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].canonical_path(), "users");
    }

    /// Spec `models.md` §"Model naming" + §Design: in a single-model file the
    /// file stem is **always** authoritative; the `name:` frontmatter key is
    /// accepted but has no effect on the model's identity. A file
    /// `daily_revenue.sql` declaring `name: something_else` must still resolve
    /// to the model name `daily_revenue` (and canonical path `daily_revenue`),
    /// not `something_else`.
    #[test]
    fn single_model_frontmatter_name_does_not_override_file_stem() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let content = "---\nname: something_else\nmaterialization: table\n---\nSELECT 1 AS x";
        std::fs::File::create(models_dir.join("daily_revenue.sql"))
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].name, "daily_revenue",
            "single-model identity must come from the file stem, not frontmatter name:"
        );
        assert_eq!(
            models[0].canonical_path(),
            "daily_revenue",
            "canonical path leaf must be the file stem, not frontmatter name:"
        );
    }

    // ----- P1 address strip-list tests (D-01) --------------------------------

    /// A file directly in the project root yields a single bare-name segment.
    /// Spec: architecture.md §"Resolution" — root file → `smelt.<stem>`.
    #[test]
    fn address_root_level_file_is_bare_name() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("foo.sql");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"SELECT 1")
            .unwrap();

        let segs = ModelDiscovery::compute_address_segments(&file_path, dir.path(), &[]);
        assert_eq!(segs, vec!["foo"]);
    }

    /// Under `paths: ["models"]`, a file inside the prefix gets the prefix
    /// stripped; a file outside keeps its full project-relative path.
    /// Spec: architecture.md §"Resolution" + smelt_yml.md §"Top-level keys" (Semantics 5).
    #[test]
    fn address_strips_only_configured_prefix() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models").join("marts");
        let sources_dir = dir.path().join("sources").join("raw");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::create_dir_all(&sources_dir).unwrap();

        let inside = models_dir.join("x.sql");
        let outside = sources_dir.join("events.yml");
        for p in [&inside, &outside] {
            std::fs::File::create(p).unwrap().write_all(b"").unwrap();
        }

        let paths = vec!["models".to_string()];
        let inside_segs = ModelDiscovery::compute_address_segments(&inside, dir.path(), &paths);
        let outside_segs = ModelDiscovery::compute_address_segments(&outside, dir.path(), &paths);

        assert_eq!(inside_segs, vec!["marts", "x"]);
        assert_eq!(outside_segs, vec!["sources", "raw", "events"]);
    }

    /// With multiple `paths:` entries, each strips independently — two files with
    /// the same stem in different `paths:` dirs resolve to the same address segments.
    /// Spec: architecture.md §"Resolution" — multiple prefixes stripped independently.
    #[test]
    fn address_multi_prefix_independent_strip() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        let fixtures_dir = dir.path().join("fixtures");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::create_dir_all(&fixtures_dir).unwrap();

        let a = models_dir.join("users.sql");
        let b = fixtures_dir.join("users.sql");
        for p in [&a, &b] {
            std::fs::File::create(p)
                .unwrap()
                .write_all(b"SELECT 1")
                .unwrap();
        }

        let paths = vec!["models".to_string(), "fixtures".to_string()];
        let segs_a = ModelDiscovery::compute_address_segments(&a, dir.path(), &paths);
        let segs_b = ModelDiscovery::compute_address_segments(&b, dir.path(), &paths);

        // Both strip to the same leaf — the collision that DuplicateAddress catches
        assert_eq!(segs_a, vec!["users"]);
        assert_eq!(segs_b, vec!["users"]);
    }

    /// A generator file (`generates: models` frontmatter) whose filename does
    /// NOT end with `.gen.sql` must return zero parse errors from `parse_sql_file`.
    ///
    /// Spec: meta_language.md §"Multi-model production" — "The `.gen.sql` extension
    /// is a recommended convention; it is **not load-bearing**. The compiler
    /// determines a file's status from the frontmatter alone."
    ///
    /// Before the fix this test red-paths: `parse_sql_file` fell into the `_ =>`
    /// arm, parsed the meta-expression body as SQL, and returned parse errors.
    #[test]
    fn generator_file_without_gen_suffix_has_no_parse_errors() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // A generator file named with a plain `.sql` extension (no `.gen.`).
        let content = r#"---
generates: models
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT 1 AS x
     })
"#;
        let file_path = models_dir.join("cohort_generator.sql");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();

        let models = parse_sql_file(&file_path, None).unwrap();
        assert!(
            models.is_empty() || models.iter().all(|m| m.parse_errors.is_empty()),
            "generator file without .gen.sql suffix must produce no parse errors; got: {:?}",
            models
                .iter()
                .flat_map(|m| m.parse_errors.iter())
                .collect::<Vec<_>>()
        );
    }

    // ----- P2 universal discovery tests (D-01, D-05) -----------------------

    /// A bare-SELECT .sql under `billing/staging/` (not in `paths: ["models"]`)
    /// must be discovered and addressed `billing.staging.<stem>`.
    /// Spec: architecture.md §"Resolution" — project-wide discovery, kind by content.
    #[test]
    fn discovers_model_outside_models_dir() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let billing_dir = dir.path().join("billing").join("staging");
        std::fs::create_dir_all(&billing_dir).unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        // Write smelt.yml with paths: ["models"] — billing/ is NOT in paths
        std::fs::write(
            dir.path().join("smelt.yml"),
            "name: t\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        let sql_path = billing_dir.join("invoices.sql");
        std::fs::File::create(&sql_path)
            .unwrap()
            .write_all(b"SELECT 1 AS id")
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        let invoice = models.iter().find(|m| m.name == "invoices");
        assert!(
            invoice.is_some(),
            "invoices.sql outside models/ must be discovered; got: {:?}",
            models.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
        assert_eq!(
            invoice.unwrap().address_segments,
            vec!["billing", "staging", "invoices"],
            "address must include full path because billing/ is not a paths prefix"
        );
    }

    /// A `smelt.define` .sql under `random/x.sql` (not in `functions/`) must be
    /// discovered as a function with address `random.x`.
    /// Spec: architecture.md §"Resolution" — no dedicated `functions/` gate.
    #[test]
    fn discovers_function_anywhere() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let random_dir = dir.path().join("random");
        std::fs::create_dir_all(&random_dir).unwrap();
        let fn_path = random_dir.join("helper.sql");
        std::fs::File::create(&fn_path)
            .unwrap()
            .write_all(b"smelt.define helper() AS (SELECT 1)")
            .unwrap();

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();
        let found = models.iter().find(|m| m.name == "helper");
        assert!(
            found.is_some(),
            "helper.sql in random/ must be discovered; got: {:?}",
            models.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
        assert_eq!(
            found.unwrap().address_segments,
            vec!["random", "helper"],
            "address must be random.helper (no functions/ prefix stripped)"
        );
    }

    /// Files under hidden directories (`.git`, `.smelt`) and `target/` must NOT
    /// be discovered by the universal walk.
    /// Spec: architecture.md §"Resolution" — exclusion skip-list.
    #[test]
    fn excludes_hidden_and_target_dirs() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // A normal model that SHOULD be discovered
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::File::create(models_dir.join("good.sql"))
            .unwrap()
            .write_all(b"SELECT 1")
            .unwrap();

        // Files under hidden and target dirs that SHOULD NOT be discovered
        for excluded in [".git", ".smelt", "target"] {
            let excluded_dir = dir.path().join(excluded);
            std::fs::create_dir_all(&excluded_dir).unwrap();
            std::fs::File::create(excluded_dir.join("hidden.sql"))
                .unwrap()
                .write_all(b"SELECT 1")
                .unwrap();
        }

        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec!["models".to_string()]);
        let models = discovery.discover_models().unwrap();

        // Should only find good.sql
        assert!(
            models.iter().any(|m| m.name == "good"),
            "good.sql must be discovered"
        );
        assert!(
            !models.iter().any(|m| m.name == "hidden"),
            "files in .git/.smelt/target/ must not be discovered; got: {:?}",
            models.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }
}
