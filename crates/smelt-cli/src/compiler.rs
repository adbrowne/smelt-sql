use crate::config::{Config, Materialization};
use crate::discovery::ModelFile;
use crate::errors::{extract_snippet, text_range_to_line_col, CliError};
use anyhow::{anyhow, Result};
use rowan::TextRange;

#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub name: String,
    pub sql: String,
    pub materialization: Materialization,
}

/// Replace smelt.ref() calls with qualified table names using AST-based ranges.
///
/// This function performs byte-exact replacements using TextRange positions from the parser.
/// Refs are processed from end to start to avoid offset shifting.
fn replace_refs_with_ranges(
    sql: &str,
    refs: &[(String, TextRange)], // (model_name, range)
    schema: &str,
) -> String {
    // Sort by position (descending) to avoid offset shifting
    let mut sorted: Vec<_> = refs.iter().collect();
    sorted.sort_by(|a, b| b.1.start().cmp(&a.1.start()));

    let mut result = sql.to_string();
    for (model_name, range) in sorted {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        let replacement = format!("{}.{}", schema, model_name);
        result.replace_range(start..end, &replacement);
    }

    result
}

pub struct SqlCompiler {
    config: Config,
}

impl SqlCompiler {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Compile a model's SQL by replacing smelt.ref() calls with table references
    pub fn compile(&self, model: &ModelFile, schema: &str) -> Result<CompiledModel> {
        // ERROR if any named parameters detected
        for ref_info in &model.refs {
            if ref_info.has_named_params {
                let (line, col) = text_range_to_line_col(&model.content, ref_info.range);
                let snippet = extract_snippet(&model.content, ref_info.range, 0);

                return Err(CliError::NamedParametersNotSupported {
                    model: model.name.clone(),
                    file: model.path.clone(),
                    line,
                    col,
                    snippet,
                }
                .into());
            }
        }

        // Prepare refs for AST-based replacement
        let refs: Vec<(String, TextRange)> = model
            .refs
            .iter()
            .map(|r| (r.model_name.clone(), r.range))
            .collect();

        // Use AST-based replacement with precise byte offsets
        let compiled_sql = replace_refs_with_ranges(&model.content, &refs, schema);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Compile a model with custom SQL (e.g., for transformed queries).
    /// This is used for incremental processing where the SQL has been transformed.
    pub fn compile_with_sql(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
    ) -> Result<CompiledModel> {
        // Reparse transformed SQL to get accurate ref positions
        // (byte offsets change after inject_time_filter transforms the SQL)
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::File::cast(parse.syntax())
            .ok_or_else(|| anyhow!("Failed to parse transformed SQL"))?;

        // Extract refs with their ranges from transformed SQL
        let refs: Vec<(String, TextRange)> = file
            .refs()
            .filter_map(|ref_call| {
                let name = ref_call.model_name()?;
                let range = ref_call.range();
                Some((name, range))
            })
            .collect();

        // Use AST-based replacement with precise byte offsets
        let compiled_sql = replace_refs_with_ranges(sql, &refs, schema);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: compiled_sql,
            materialization,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelConfig, Target};
    use crate::discovery::RefInfo;
    use std::collections::HashMap;

    /// Helper function to parse SQL and extract refs with real TextRange values
    fn extract_refs_from_sql(sql: &str) -> Vec<RefInfo> {
        let parse = smelt_parser::parse(sql);
        if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
            file.refs()
                .filter_map(|ref_call| {
                    Some(RefInfo {
                        model_name: ref_call.model_name()?,
                        has_named_params: ref_call.named_params().count() > 0,
                        range: ref_call.range(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn make_test_config() -> Config {
        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    #[test]
    fn test_simple_ref_replacement() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.ref('raw_events')
GROUP BY user_id
"#;

        let model = ModelFile {
            name: "user_stats".to_string(),
            path: "models/user_stats.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.raw_events"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT a.user_id, b.session_id
FROM smelt.ref('model_a') a
JOIN smelt.ref('model_b') b ON a.id = b.id
"#;

        let model = ModelFile {
            name: "combined".to_string(),
            path: "models/combined.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a a"));
        assert!(compiled.sql.contains("JOIN main.model_b b"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_named_params_error() {
        let sql = r#"
SELECT user_id
FROM smelt.ref('raw_events', filter => event_type = 'page_view')
"#;

        let model = ModelFile {
            name: "filtered".to_string(),
            path: "models/filtered.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let result = compiler.compile(&model, "main");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("named parameters"));
        assert!(err_msg.contains("not yet supported"));
    }

    #[test]
    fn test_materialization_from_config() {
        let model = ModelFile {
            name: "test_model".to_string(),
            path: "models/test_model.sql".into(),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: None,
                tags: Vec::new(),
            },
        );

        let compiler = SqlCompiler::new(config);
        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(matches!(compiled.materialization, Materialization::Table));
    }

    #[test]
    fn test_ref_with_double_quotes() {
        let sql = r#"SELECT * FROM smelt.ref("model_a")"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_ref_with_whitespace() {
        let sql = r#"SELECT * FROM smelt.ref( 'model_a' )"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_multiple_refs_same_model() {
        let sql = r#"
SELECT a.id, b.id
FROM smelt.ref('model_a') a
JOIN smelt.ref('model_a') b ON a.parent_id = b.id
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        // Both instances should be replaced
        assert_eq!(compiled.sql.matches("main.model_a").count(), 2);
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_refs_preserve_formatting() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as count
FROM smelt.ref('events')
WHERE event_type = 'click'
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config);

        let compiled = compiler.compile(&model, "main").unwrap();

        // Verify formatting is preserved (newlines, indentation)
        assert!(compiled.sql.contains("SELECT\n    user_id,"));
        assert!(compiled.sql.contains("FROM main.events"));
        assert!(compiled.sql.contains("WHERE event_type = 'click'"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }
}
