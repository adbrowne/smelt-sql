use crate::errors::CliError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Materialization {
    Table,
    View,
}

impl<'de> Deserialize<'de> for Materialization {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "table" => Ok(Materialization::Table),
            "view" => Ok(Materialization::View),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid materialization type: {}. Must be 'table' or 'view'",
                s
            ))),
        }
    }
}

impl Serialize for Materialization {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Materialization::Table => serializer.serialize_str("table"),
            Materialization::View => serializer.serialize_str("view"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub name: String,
    pub version: u32,
    #[serde(default = "default_model_paths")]
    pub model_paths: Vec<String>,
    pub targets: HashMap<String, Target>,
    #[serde(default = "default_materialization")]
    pub default_materialization: Materialization,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
    /// Path to Python interpreter (overridden by SMELT_PYTHON env var)
    #[serde(default)]
    pub python: Option<String>,
}

fn default_model_paths() -> Vec<String> {
    vec!["models".to_string()]
}

fn default_materialization() -> Materialization {
    Materialization::View
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
    #[serde(rename = "type")]
    pub target_type: String,
    // DuckDB fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub schema: String,
    // Spark fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
}

impl Target {
    /// Get the backend type from the target_type field.
    pub fn backend_type(&self) -> BackendType {
        match self.target_type.to_lowercase().as_str() {
            "duckdb" => BackendType::DuckDB,
            "spark" => BackendType::Spark,
            _ => BackendType::DuckDB, // Default to DuckDB for backward compatibility
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    DuckDB,
    Spark,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub materialization: Option<Materialization>,
    #[serde(default)]
    pub incremental: Option<IncrementalConfig>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct IncrementalConfig {
    pub enabled: bool,
    /// Column in source data to filter on (for WHERE injection)
    pub event_time_column: String,
    /// Column in output to delete by (for DELETE+INSERT)
    pub partition_column: String,
}

impl Config {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join("smelt.yml");
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| CliError::ConfigLoadError {
                path: config_path.clone(),
                source: e.into(),
            })?;

        serde_yaml::from_str(&content).map_err(|e| {
            CliError::ConfigLoadError {
                path: config_path,
                source: e.into(),
            }
            .into()
        })
    }

    /// Get materialization for a model
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config > default_materialization
    pub fn get_materialization(&self, model_name: &str) -> Materialization {
        self.models
            .get(model_name)
            .and_then(|m| m.materialization.clone())
            .unwrap_or_else(|| self.default_materialization.clone())
    }

    /// Get materialization with SQL metadata precedence
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config > default_materialization
    pub fn get_materialization_with_metadata(
        &self,
        model_name: &str,
        sql_metadata: Option<&crate::metadata::ModelMetadata>,
    ) -> Materialization {
        // Check SQL metadata first
        if let Some(metadata) = sql_metadata {
            if let Some(materialization) = &metadata.materialization {
                return materialization.clone();
            }
        }

        // Fall back to smelt.yml
        self.get_materialization(model_name)
    }

    /// Get incremental config for a model if enabled
    ///
    /// **Precedence**: smelt.yml only (for now)
    pub fn get_incremental(&self, model_name: &str) -> Option<&IncrementalConfig> {
        self.models
            .get(model_name)
            .and_then(|m| m.incremental.as_ref())
            .filter(|i| i.enabled)
    }

    /// Get merged tags for a model (union of smelt.yml + frontmatter, fully deduplicated)
    pub fn get_tags(
        &self,
        model_name: &str,
        metadata: Option<&crate::metadata::ModelMetadata>,
    ) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut tags: Vec<String> = Vec::new();

        // Add tags from smelt.yml model config
        if let Some(model_config) = self.models.get(model_name) {
            for tag in &model_config.tags {
                if seen.insert(tag.clone()) {
                    tags.push(tag.clone());
                }
            }
        }

        // Add tags from SQL frontmatter
        if let Some(meta) = metadata {
            for tag in &meta.tags {
                if seen.insert(tag.clone()) {
                    tags.push(tag.clone());
                }
            }
        }

        tags
    }

    /// Get incremental config with SQL metadata precedence
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config
    pub fn get_incremental_with_metadata<'a>(
        &'a self,
        model_name: &str,
        sql_metadata: Option<&'a crate::metadata::ModelMetadata>,
    ) -> Option<&'a IncrementalConfig> {
        // Check SQL metadata first
        if let Some(metadata) = sql_metadata {
            if let Some(ref incremental) = metadata.incremental {
                if incremental.enabled {
                    return Some(incremental);
                }
            }
        }

        // Fall back to smelt.yml
        self.get_incremental(model_name)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SourceConfig {
    pub version: u32,
    pub sources: HashMap<String, SourceSchema>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SourceSchema {
    pub tables: HashMap<String, SourceTable>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SourceTable {
    #[serde(default)]
    pub description: String,
    pub columns: Vec<SourceColumn>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SourceColumn {
    pub name: String,
    #[serde(rename = "type")]
    column_type_str: String,
    #[serde(default)]
    pub description: String,
}

impl SourceColumn {
    /// Create a new SourceColumn
    pub fn new(name: String, column_type: String, description: String) -> Self {
        Self {
            name,
            column_type_str: column_type,
            description,
        }
    }

    /// Get the column type as a string (for backwards compatibility)
    pub fn column_type_str(&self) -> &str {
        &self.column_type_str
    }

    /// Get the parsed DataType
    pub fn data_type(&self) -> Option<smelt_types::DataType> {
        smelt_types::parse_type(&self.column_type_str).ok()
    }
}

impl SourceConfig {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let sources_path = project_dir.join("sources.yml");
        let content =
            std::fs::read_to_string(&sources_path).map_err(|e| CliError::ConfigLoadError {
                path: sources_path.clone(),
                source: e.into(),
            })?;

        serde_yaml::from_str(&content).map_err(|e| {
            CliError::ConfigLoadError {
                path: sources_path,
                source: e.into(),
            }
            .into()
        })
    }

    /// Get full source name (schema.table format)
    pub fn get_source_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (schema_name, schema) in &self.sources {
            for table_name in schema.tables.keys() {
                names.push(format!("{}.{}", schema_name, table_name));
            }
        }
        names
    }
}

/// Find the smelt project root by looking for smelt.yml or models/ directory
pub fn find_project_root(start_dir: &Path) -> Result<PathBuf> {
    let mut current = start_dir.to_path_buf();

    // Walk up max 5 levels
    for _ in 0..5 {
        // Check for smelt.yml
        if current.join("smelt.yml").exists() {
            return Ok(current);
        }

        // Check for models/ directory
        if current.join("models").is_dir() {
            return Ok(current);
        }

        // Go up one level
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err(CliError::ProjectRootNotFound.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_materialization_deserialization() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  model1:
    materialization: table
  model2:
    materialization: view
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "test_project");
        assert_eq!(
            config.models.get("model1").unwrap().materialization,
            Some(Materialization::Table)
        );
        assert_eq!(
            config.models.get("model2").unwrap().materialization,
            Some(Materialization::View)
        );
    }

    #[test]
    fn test_default_materialization() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.default_materialization, Materialization::View);
    }
}
