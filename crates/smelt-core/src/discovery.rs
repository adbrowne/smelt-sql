use crate::metadata::{extract_file_metadata, FileMetadata, ModelMetadata};
use crate::refs::extract_refs;
pub use crate::refs::RefInfo;
use anyhow::{anyhow, Context, Result};
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ModelFile {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
    pub refs: Vec<RefInfo>,
    pub parse_errors: Vec<smelt_parser::ParseError>,
    /// Metadata extracted from YAML frontmatter
    pub metadata: Option<Box<ModelMetadata>>,
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
                    let model = self.parse_model_file(path)?;
                    models.push(model);
                }
            }
        }

        if models.is_empty() {
            return Err(anyhow!(
                "No models found in model paths: {}",
                self.model_paths.join(", ")
            ));
        }

        Ok(models)
    }

    fn parse_model_file(&self, path: &Path) -> Result<ModelFile> {
        // Read file content
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read model file: {:?}", path))?;

        // Extract metadata from YAML frontmatter
        let file_metadata = extract_file_metadata(&content).ok();
        let model_metadata = match file_metadata {
            Some(FileMetadata::Single { metadata, .. }) => Some(metadata),
            Some(FileMetadata::Multi { models }) => {
                // For multi-model files, we need to handle each model separately
                // For now, just use the first model's metadata if it matches the filename
                let filename_stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());

                models
                    .into_iter()
                    .find(|section| section.metadata.name.as_ref() == filename_stem.as_ref())
                    .map(|section| Box::new(section.metadata))
            }
            Some(FileMetadata::Empty) | None => None,
        };

        // Determine model name: from metadata if present, otherwise from filename
        let name = model_metadata
            .as_ref()
            .and_then(|m| m.name.clone())
            .or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .ok_or_else(|| anyhow!("Cannot determine model name from {:?}", path))?;

        // Parse using smelt-parser
        let parse = smelt_parser::parse(&content);

        // Extract refs using AST
        let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
            extract_refs(&file)
        } else {
            Vec::new()
        };

        Ok(ModelFile {
            name,
            path: path.to_path_buf(),
            content,
            refs,
            parse_errors: parse.errors,
            metadata: model_metadata,
        })
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
