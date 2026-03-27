use anyhow::{anyhow, Context, Result};
pub use smelt_core::extract_refs;
pub use smelt_core::RefInfo;
use smelt_core::{
    extract_file_metadata, FileMetadata, Materialization, ModelId, ModelMetadata, TestConfig,
};
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::python::PythonModelQuery;

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

pub struct ModelDiscovery {
    project_root: PathBuf,
    model_paths: Vec<String>,
}

impl ModelDiscovery {
    pub fn new(project_root: PathBuf, model_paths: Vec<String>) -> Self {
        Self {
            project_root,
            model_paths,
        }
    }

    /// Discover SQL model files under the configured model paths.
    /// Returns a list of [`ModelFile`] instances representing the discovered SQL models.
    pub fn discover_models(&self) -> Result<Vec<ModelFile>> {
        let mut models = Vec::new();

        for model_path in &self.model_paths {
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

        Ok(models)
    }

    /// Scan model paths for Python files containing `@model` decorators.
    /// Returns (file_path, decorator_line_numbers, file_content) tuples.
    pub fn discover_python_files(&self) -> Result<Vec<(PathBuf, Vec<usize>, String)>> {
        let mut python_files = Vec::new();

        for model_path in &self.model_paths {
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

                    let decorator_lines = crate::python::scan_for_model_decorators(&content);

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
FROM smelt.ref('raw_events')
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
        let sql = r#"
SELECT user_id
FROM smelt.ref('raw_events', filter => event_type = 'page_view')
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].model_name, "raw_events");
        assert!(refs[0].has_named_params);
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT
    a.user_id,
    b.session_id
FROM smelt.ref('model_a') a
INNER JOIN smelt.ref('model_b') b ON a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
    }
}
