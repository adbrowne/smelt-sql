use crate::config::{BackendType, Config, Materialization, Target};
use crate::discovery::ModelFile;
use crate::errors::{extract_snippet, text_range_to_line_col, CliError};
use anyhow::Result;
use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_dialect::{wrap_with_type_casts, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;
use smelt_types::DataType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub name: String,
    pub sql: String,
    pub materialization: Materialization,
}

fn dialect_for_backend(backend_type: BackendType) -> (SqlDialect, BackendCapabilities) {
    match backend_type {
        BackendType::DuckDB => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        BackendType::Spark => (SqlDialect::SparkSQL, BackendCapabilities::spark()),
    }
}

/// Resolve all smelt.ref() and smelt.source() calls in arbitrary SQL text by replacing
/// them with qualified table names.
pub fn resolve_refs_in_sql(sql: &str, schema: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let ctx = PrintContext {
        dialect: &SqlDialect::DuckDB,
        capabilities: &BackendCapabilities::duckdb(),
        schema,
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

pub struct SqlCompiler {
    config: Config,
    dialect: SqlDialect,
    capabilities: BackendCapabilities,
}

impl SqlCompiler {
    pub fn new(config: Config, target: &Target) -> Self {
        let (dialect, capabilities) = dialect_for_backend(target.backend_type());
        Self {
            config,
            dialect,
            capabilities,
        }
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

        // Strip frontmatter to avoid parse errors from YAML metadata
        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let parse = smelt_parser::parse(&clean_content);
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Type-conforming cast insertion: wrap SELECT columns with CASTs so
        // backend output types match smelt's type inference exactly.
        let compiled_sql = self.apply_type_casts(&compiled_sql);

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

    /// Wrap SELECT columns with CASTs based on type inference.
    ///
    /// Returns the original SQL unchanged if type inference can't extract
    /// column names/types (e.g. models referencing other models via smelt.ref()
    /// where upstream schemas aren't yet available).
    fn apply_type_casts(&self, sql: &str) -> String {
        let parse = smelt_parser::parse(sql);
        let file = match File::cast(parse.syntax()) {
            Some(f) => f,
            None => return sql.to_string(),
        };
        let select_stmt = match file.select_stmt() {
            Some(s) => s,
            None => return sql.to_string(),
        };

        let ctx = TypeContext::new();
        let column_types = infer_select_column_types(&select_stmt, &ctx);

        let select_list = match select_stmt.select_list() {
            Some(sl) => sl,
            None => return sql.to_string(),
        };
        let items: Vec<_> = select_list.items().collect();

        // Only apply casts if we have concrete types for at least one column
        let has_concrete = column_types
            .iter()
            .any(|tc| !matches!(tc.data_type, DataType::Unknown | DataType::Null));
        if !has_concrete {
            return sql.to_string();
        }

        let col_names: Vec<String> = items
            .iter()
            .map(|item| item.alias().unwrap_or_else(|| "?".to_string()))
            .collect();
        let col_name_refs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
        let col_type_refs: Vec<DataType> =
            column_types.iter().map(|tc| tc.data_type.clone()).collect();

        wrap_with_type_casts(sql, &col_name_refs, &col_type_refs)
    }

    /// Compile a model with custom SQL (e.g., for transformed queries).
    /// This is used for incremental processing where the SQL has been transformed.
    pub fn compile_with_sql(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
    ) -> Result<CompiledModel> {
        let parse = smelt_parser::parse(sql);
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

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

/// Registry of SQL compilers, one per target.
///
/// Each target may have a different dialect (DuckDB vs Spark), so we need
/// one compiler per target to emit correct SQL.
pub struct CompilerRegistry {
    compilers: HashMap<String, SqlCompiler>,
}

impl CompilerRegistry {
    /// Create compilers for all targets in the set.
    pub fn new(config: &Config, targets: &HashMap<String, Target>) -> Self {
        let mut compilers = HashMap::new();
        for (name, target) in targets {
            compilers.insert(name.clone(), SqlCompiler::new(config.clone(), target));
        }
        Self { compilers }
    }

    /// Get the compiler for a target name.
    pub fn get(&self, target_name: &str) -> &SqlCompiler {
        &self.compilers[target_name]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelConfig, Target};
    use crate::discovery::RefInfo;
    use std::collections::HashMap;

    fn make_test_target() -> Target {
        Target {
            target_type: "duckdb".to_string(),
            database: Some("test.duckdb".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
        }
    }

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
            seed_paths: vec!["seeds".to_string()],
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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: None,
                tags: Vec::new(),
                target: None,
            },
        );

        let compiler = SqlCompiler::new(config, &make_test_target());
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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

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
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Verify formatting is preserved (newlines, indentation)
        assert!(compiled.sql.contains("SELECT\n    user_id,"));
        assert!(compiled.sql.contains("FROM main.events"));
        assert!(compiled.sql.contains("WHERE event_type = 'click'"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }
}
