use crate::config::Materialization;
use crate::metadata::{extract_file_metadata, FileMetadata, ModelMetadata, TestConfig};
use crate::model_id::ModelId;
use crate::refs::extract_refs;
pub use crate::refs::RefInfo;
use anyhow::{anyhow, Context, Result};
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
    /// Canonical model identifier.
    pub model_id: ModelId,
}

impl ModelFile {
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
}

/// Discover `smelt.define` / `smelt.extern` files under the project-root
/// `functions/` directory.
///
/// Returns the paths of every `.sql` file beneath `<project_root>/functions/`
/// (recursive). The `functions/` directory is hardcoded and not currently
/// configurable in `smelt.yml`; a configurable discovery policy is deferred
/// per §21 of the smelt-functions research.
///
/// Returns an empty vector if `functions/` does not exist. Both the CLI's
/// `Discovery::discover_function_files` and the LSP's `initialize` call this
/// to keep their function-discovery behavior in sync.
pub fn discover_function_file_paths(project_root: &Path) -> Vec<PathBuf> {
    let search_path = project_root.join("functions");
    if !search_path.exists() {
        return Vec::new();
    }

    let mut paths = Vec::new();
    for entry in WalkDir::new(&search_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            paths.push(path.to_path_buf());
        }
    }
    paths
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
        let mut models = Vec::new();

        for model_path in &self.paths {
            let search_path = self.project_root.join(model_path);

            if !search_path.exists() {
                continue;
            }

            // Recursively find all .sql files
            for entry in WalkDir::new(&search_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    let parsed = self.parse_model_file(path)?;
                    models.extend(parsed);
                }
            }
        }

        if models.is_empty() {
            return Err(anyhow!(
                "No models found in paths: {}",
                self.paths.join(", ")
            ));
        }

        Ok(models)
    }

    fn parse_model_file(&self, path: &Path) -> Result<Vec<ModelFile>> {
        // Read file content
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read model file: {:?}", path))?;

        // Extract metadata from YAML frontmatter
        let file_metadata = match extract_file_metadata(&content) {
            Ok(fm) => Some(fm),
            Err(e) => {
                eprintln!("Warning: {}: {}", path.display(), e);
                None
            }
        };

        match file_metadata {
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
                        model_id,
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

                let name = model_metadata
                    .as_ref()
                    .and_then(|m| m.name.clone())
                    .or_else(|| {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())
                    })
                    .ok_or_else(|| anyhow!("Cannot determine model name from {:?}", path))?;

                let parse = smelt_parser::parse(&content);
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
                    model_id,
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
        assert_eq!(refs[0].model_name, "raw_events");
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
        assert_eq!(refs[0].model_name, "format_date");
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
        assert_eq!(cleaned.refs[0].model_name, "staging_events");

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
            .write_all(b"-- top-level fn")
            .unwrap();
        let nested = nested_dir.join("inner.sql");
        std::fs::File::create(&nested)
            .unwrap()
            .write_all(b"-- nested fn")
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
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
    }
}
