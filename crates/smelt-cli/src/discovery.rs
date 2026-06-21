use anyhow::{anyhow, Context, Result};
pub use smelt_core::extract_refs;
pub use smelt_core::RefInfo;
use smelt_core::{
    extract_file_metadata, parse_sql_file, FileMetadata, Materialization, ModelId, ModelMetadata,
};
use smelt_db::EmittedModelDef;
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// `ModelFile`, `ModelKind`, and `PythonModelQuery` are the canonical
// definitions in `smelt-core`. Re-exported here so existing CLI imports
// (`use crate::discovery::ModelFile`) continue to work unchanged.
pub use smelt_core::{ModelFile, ModelKind};

/// Construct a virtual `ModelFile` from an `EmittedModelDef` survivor.
///
/// The `smelt_name` parameter is the pre-computed smelt-path for this
/// emitted model (e.g. `"cohorts.us_west"`), obtained from
/// `smelt_db::emitted_model_smelt_path`. The emitted model's SQL body is
/// used as the model content and is parsed for refs.
///
/// The resulting `ModelFile` has a virtual `path` derived from the
/// generator file path (since emitted models have no physical SQL file of
/// their own). Its `metadata` carries the materialization and tags from
/// the `EmittedModelDef`.
///
/// Free function (rather than `impl ModelFile`) because `EmittedModelDef`
/// lives in `smelt-db`, which depends on `smelt-core` — placing the
/// constructor here keeps the dependency direction clean.
pub fn model_file_from_emitted_def(emitted: &EmittedModelDef, smelt_name: String) -> ModelFile {
    // Parse the body text for ref extraction.
    let content = emitted.body_text.clone();
    let parse = smelt_parser::parse(&content);
    let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
        extract_refs(&file)
    } else {
        Vec::new()
    };

    // Build minimal metadata from emitted fields.
    // Note: "incremental" is not a Materialization variant — it's a Table
    // with an IncrementalConfig attached.
    let materialization = match emitted.materialization.as_str() {
        "table" | "incremental" => Some(Materialization::Table),
        _ => Some(Materialization::View),
    };
    let metadata = Box::new(ModelMetadata {
        name: Some(smelt_name.clone()),
        generates: None,
        materialization,
        timeseries: emitted.timeseries_config.clone(),
        incremental: emitted.incremental_config.clone(),
        target: None,
        tags: emitted.tags.clone(),
        owner: None,
        description: if emitted.description.is_empty() {
            None
        } else {
            Some(emitted.description.clone())
        },
        columns: std::collections::HashMap::new(),
        backend_hints: std::collections::HashMap::new(),
        test: None,
        schema_evolution: None,
        format: None,
        reuse: None,
        forward_only: false,
        state: None,
    });

    // Virtual path: generator_file path with the model name appended as
    // a virtual component so the Salsa key is unique per emission.
    let virtual_path = emitted.generator_file.with_file_name(format!(
        "{}::{}",
        emitted
            .generator_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("gen"),
        smelt_name
    ));
    let model_id = ModelId::multi_model(emitted.generator_file.clone(), smelt_name.clone());

    // Address segments: smelt_name split by '.'
    let address_segments: Vec<String> = smelt_name.split('.').map(|s| s.to_string()).collect();

    ModelFile {
        name: smelt_name,
        path: virtual_path,
        content,
        refs,
        parse_errors: parse.errors,
        metadata: Some(metadata),
        kind: ModelKind::Sql,
        model_id,
        address_segments,
    }
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

    /// Discover SQL model files under the configured model paths.
    /// Returns a list of [`ModelFile`] instances representing the discovered SQL models.
    pub fn discover_models(&self) -> Result<Vec<ModelFile>> {
        let mut models = Vec::new();

        for model_path in &self.paths {
            let search_path = self.project_root.join(model_path);

            if !search_path.exists() {
                continue;
            }

            // The scan root for address computation is project_root / model_path.
            let scan_root = search_path.clone();

            // Recursively find all .sql files
            for entry in WalkDir::new(&search_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    let mut parsed = self.parse_model_file(path)?;
                    // Compute address_segments: path relative to scan_root,
                    // parent directory components + leaf model name.
                    let address_segments: Vec<String> =
                        Self::compute_address_segments(path, &scan_root);
                    for m in &mut parsed {
                        m.address_segments = address_segments.clone();
                        // For multi-model files, keep the model's declared name
                        // as the leaf segment instead of the file stem.
                        if let Some(last) = m.address_segments.last_mut() {
                            *last = m.name.clone();
                        }
                    }
                    models.extend(parsed);
                }
            }
        }

        Ok(models)
    }

    /// Compute address segments for a file at `path` with the given `scan_root`.
    ///
    /// For `models/staging/stg_events.sql` with `scan_root = models/`:
    ///   dir_segments = ["staging"], leaf = file stem = "stg_events"
    ///   → ["staging", "stg_events"]
    fn compute_address_segments(path: &Path, scan_root: &Path) -> Vec<String> {
        let Ok(rel) = path.strip_prefix(scan_root) else {
            return Vec::new();
        };
        let parent = rel.parent().unwrap_or(std::path::Path::new(""));
        let mut segs: Vec<String> = parent
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        // Leaf: file stem (will be replaced with model name for multi-model files).
        if let Some(stem) = rel
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        {
            segs.push(stem);
        }
        segs
    }

    /// Discover `smelt.define` / `smelt.extern` files under the project-root
    /// `functions/` directory.
    ///
    /// Function files are loaded into Salsa the same way as model files (each
    /// becomes a [`ModelFile`]) so that `smelt-db`'s `functions_in_file` query
    /// can index their signatures. The path-discovery primitive lives in
    /// `smelt_core::discover_function_file_paths` so the LSP can share it.
    pub fn discover_function_files(&self) -> Result<Vec<ModelFile>> {
        let mut files = Vec::new();
        for path in smelt_core::discover_function_file_paths(&self.project_root) {
            // Use `parse_sql_file` with `project_root` as the scan root so
            // that `address_segments` retains the full workspace-relative path
            // (e.g. `["functions", "patterns", "sessionize"]` for
            // `functions/patterns/sessionize.sql`). This mirrors what
            // `smelt_core::workspace::load_workspace` does — required by the
            // Workspace Loading Parity rule (CLI ↔ LSP).
            match parse_sql_file(&path, Some(&self.project_root)) {
                Ok(parsed) => files.extend(parsed),
                Err(e) => {
                    tracing::warn!("Failed to parse function file {}: {}", path.display(), e);
                }
            }
        }
        Ok(files)
    }

    /// Scan model paths for Python files containing `@model` decorators.
    /// Returns (file_path, decorator_line_numbers, file_content) tuples.
    pub fn discover_python_files(&self) -> Result<Vec<(PathBuf, Vec<u32>, String)>> {
        let mut python_files = Vec::new();

        for model_path in &self.paths {
            let search_path = self.project_root.join(model_path);

            if !search_path.exists() {
                continue;
            }

            for entry in WalkDir::new(&search_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                if path.extension().and_then(|s| s.to_str()) == Some("py") {
                    let content = std::fs::read_to_string(path)
                        .with_context(|| format!("Failed to read Python file: {:?}", path))?;

                    let decorator_lines =
                        smelt_core::python_utils::scan_for_model_decorators(&content);

                    if !decorator_lines.is_empty() {
                        python_files.push((path.to_path_buf(), decorator_lines, content));
                    }
                }
            }
        }

        Ok(python_files)
    }

    fn parse_model_file(&self, path: &Path) -> Result<Vec<ModelFile>> {
        // Read file content
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read model file: {:?}", path))?;

        // Extract metadata from YAML frontmatter
        let file_metadata = match extract_file_metadata(&content) {
            Ok(fm) => Some(fm),
            Err(e) => {
                tracing::warn!("{}: {}", path.display(), e);
                None
            }
        };

        match file_metadata {
            Some(FileMetadata::Generator { metadata, .. }) => {
                // Generator files (`generates: models` frontmatter) are handled by the
                // W1–W4 Salsa pipeline, not by the SQL model discovery path.  Return
                // the file's content so the Salsa DB can register it, but skip SQL
                // parsing — the body is a meta-language expression, not SQL.
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

                Ok(vec![ModelFile {
                    name,
                    path: path.to_path_buf(),
                    content,
                    refs: vec![],
                    parse_errors: vec![],
                    metadata: Some(metadata),
                    kind: ModelKind::Sql,
                    model_id: ModelId::from_path(path.to_path_buf()),
                    address_segments: Vec::new(),
                }])
            }
            Some(FileMetadata::Multi { models }) => {
                // Multi-model file: create one ModelFile per section
                let mut result = Vec::with_capacity(models.len());
                for section in models {
                    let model_name =
                        section.metadata.name.clone().ok_or_else(|| {
                            anyhow!("Multi-model section missing name in {:?}", path)
                        })?;

                    let sql_content = &content[section.sql_range.clone()];
                    let clean_content = smelt_parser::strip_frontmatter(sql_content);
                    let parse = smelt_parser::parse(&clean_content);
                    let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
                        extract_refs(&file)
                    } else {
                        Vec::new()
                    };

                    let model_id = ModelId::multi_model(path.to_path_buf(), model_name.clone());

                    result.push(ModelFile {
                        name: model_name,
                        path: model_id.salsa_key(),
                        content: sql_content.to_string(),
                        refs,
                        parse_errors: parse.errors,
                        metadata: Some(Box::new(section.metadata)),
                        kind: ModelKind::Sql,
                        model_id,
                        // Set by the caller (discover_models) after construction.
                        address_segments: Vec::new(),
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

                // Spec `models.md` §"Model naming": for single-model files the
                // file stem is *always* authoritative; the `name:` frontmatter
                // key is accepted but has no effect on identity. Mirrors
                // `smelt-core::discovery::parse_sql_file` and `smelt-db`'s
                // `parse_model` so all discovery paths agree on the name.
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("Cannot determine model name from {:?}", path))?;

                let clean_content = smelt_parser::strip_frontmatter(&content);
                let parse = smelt_parser::parse(&clean_content);
                let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
                    extract_refs(&file)
                } else {
                    Vec::new()
                };

                let model_id = ModelId::from_path(path.to_path_buf());

                Ok(vec![ModelFile {
                    name,
                    path: path.to_path_buf(),
                    content,
                    refs,
                    parse_errors: parse.errors,
                    metadata: model_metadata,
                    kind: ModelKind::Sql,
                    model_id,
                    // Set by the caller (discover_models) after construction.
                    address_segments: Vec::new(),
                }])
            }
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
        // smelt.functions.my_fn(param => value) — has_named_params=true.
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

    /// `discover_function_files` must populate `address_segments` using
    /// `project_root` as the scan root, so that the canonical path includes
    /// the `functions` prefix.
    ///
    /// `<dir>/functions/patterns/sessionize.sql` → canonical_path() ==
    /// `"functions.patterns.sessionize"`.
    #[test]
    fn cli_discover_function_files_populates_address_segments() {
        let dir = tempfile::tempdir().unwrap();
        let patterns_dir = dir.path().join("functions").join("patterns");
        std::fs::create_dir_all(&patterns_dir).unwrap();
        std::fs::write(
            patterns_dir.join("sessionize.sql"),
            "smelt.define sessionize(e: Expr<Integer>) -> Expr<Integer> AS SELECT 1",
        )
        .unwrap();

        // empty paths is fine — function discovery doesn't consult `paths:`
        let discovery = ModelDiscovery::new(dir.path().to_path_buf(), vec![]);
        let files = discovery.discover_function_files().unwrap();

        assert_eq!(files.len(), 1, "expected exactly one function file");
        assert_eq!(
            files[0].canonical_path(),
            "functions.patterns.sessionize",
            "address_segments must be populated; got {:?}",
            files[0].address_segments
        );
    }
}
