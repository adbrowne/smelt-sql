use anyhow::{anyhow, Context, Result};
use rowan::TextRange;
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
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub model_name: String,
    pub has_named_params: bool,
    pub range: TextRange,
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
        // Model name from filename (e.g., models/user_sessions.sql -> user_sessions)
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid filename: {:?}", path))?
            .to_string();

        // Read file content
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read model file: {:?}", path))?;

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
        })
    }
}

fn extract_refs(file: &AstFile) -> Vec<RefInfo> {
    file.refs()
        .filter_map(|ref_call| {
            let model_name = ref_call.model_name()?;
            let has_params = ref_call.named_params().count() > 0;
            let range = ref_call.range();

            Some(RefInfo {
                model_name,
                has_named_params: has_params,
                range,
            })
        })
        .collect()
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
FROM smelt.ref('model_a') a,
     smelt.ref('model_b') b
WHERE a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
    }
}
