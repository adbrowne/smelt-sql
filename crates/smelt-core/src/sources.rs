use crate::config::DataLatency;
use serde::Deserialize;
use smelt_types::{parse_type, DataType};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Sources configuration from sources.yml
/// Supports nested object format like dbt:
/// ```yaml
/// sources:
///   raw:
///     tables:
///       users:
///         columns: [...]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourcesConfig {
    pub sources: Vec<SourceDef>,
}

impl SourcesConfig {
    /// Load sources config from a project directory.
    /// Returns an empty config if sources.yml doesn't exist.
    pub fn load(project_dir: &Path) -> Result<Self, SourcesError> {
        let sources_path = project_dir.join("sources.yml");
        if !sources_path.exists() {
            // Also try .yaml extension
            let yaml_path = project_dir.join("sources.yaml");
            if !yaml_path.exists() {
                return Ok(Self::default());
            }
            let content =
                std::fs::read_to_string(&yaml_path).map_err(|e| SourcesError::LoadError {
                    path: yaml_path,
                    source: e.into(),
                })?;
            return serde_yaml::from_str(&content).map_err(SourcesError::ParseError);
        }

        let content =
            std::fs::read_to_string(&sources_path).map_err(|e| SourcesError::LoadError {
                path: sources_path,
                source: e.into(),
            })?;

        serde_yaml::from_str(&content).map_err(SourcesError::ParseError)
    }

    /// Find a source by name
    pub fn find_source(&self, name: &str) -> Option<&SourceDef> {
        self.sources.iter().find(|s| s.name == name)
    }

    /// Get all source names in "schema.table" format
    pub fn get_source_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for source in &self.sources {
            for table in &source.tables {
                names.push(format!("{}.{}", source.name, table.name));
            }
        }
        names
    }
}

impl<'de> Deserialize<'de> for SourcesConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Raw YAML structure with nested objects
        #[derive(Deserialize)]
        struct RawConfig {
            #[serde(default)]
            sources: HashMap<String, RawSourceDef>,
        }

        #[derive(Deserialize)]
        struct RawSourceDef {
            #[serde(default)]
            database: Option<String>,
            #[serde(default)]
            schema: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            tables: HashMap<String, RawTableDef>,
        }

        #[derive(Deserialize)]
        struct RawTableDef {
            #[serde(default)]
            identifier: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            columns: Vec<SourceColumnDef>,
        }

        let raw = RawConfig::deserialize(deserializer)?;

        let sources = raw
            .sources
            .into_iter()
            .map(|(name, raw_source)| {
                let tables = raw_source
                    .tables
                    .into_iter()
                    .map(|(table_name, raw_table)| SourceTableDef {
                        name: table_name,
                        identifier: raw_table.identifier,
                        description: raw_table.description,
                        columns: raw_table.columns,
                    })
                    .collect();

                SourceDef {
                    name,
                    database: raw_source.database,
                    schema: raw_source.schema,
                    description: raw_source.description,
                    tables,
                }
            })
            .collect();

        Ok(SourcesConfig { sources })
    }
}

/// Source definition (a named source with tables)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDef {
    pub name: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub description: Option<String>,
    pub tables: Vec<SourceTableDef>,
}

impl SourceDef {
    /// Find a table by name within this source
    pub fn find_table(&self, name: &str) -> Option<&SourceTableDef> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// Table definition within a source
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTableDef {
    pub name: String,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub columns: Vec<SourceColumnDef>,
}

/// Column definition within a source table
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColumnDef {
    pub name: String,
    pub data_type: Option<DataType>,
    pub description: Option<String>,
    /// How late data can arrive for this column (e.g., "3 days" for mobile events).
    pub data_latency: Option<DataLatency>,
}

impl<'de> Deserialize<'de> for SourceColumnDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawColumn {
            name: String,
            #[serde(default, rename = "type")]
            type_str: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            data_latency: Option<DataLatency>,
        }

        let raw = RawColumn::deserialize(deserializer)?;

        // Parse type string into DataType if present
        let data_type = raw.type_str.as_ref().and_then(|s| parse_type(s).ok());

        Ok(SourceColumnDef {
            name: raw.name,
            data_type,
            description: raw.description,
            data_latency: raw.data_latency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sources_with_data_latency() {
        let yaml = r#"
sources:
  raw:
    tables:
      transactions:
        columns:
          - name: event_time
            type: TIMESTAMP
            data_latency: "3 days"
          - name: ingestion_time
            type: TIMESTAMP
            data_latency: "0 hours"
          - name: amount
            type: DECIMAL
"#;
        let config: SourcesConfig = serde_yaml::from_str(yaml).unwrap();
        let source = config.find_source("raw").unwrap();
        let table = source.find_table("transactions").unwrap();

        let event_time = table
            .columns
            .iter()
            .find(|c| c.name == "event_time")
            .unwrap();
        assert_eq!(event_time.data_latency.as_ref().unwrap().to_days(), 3);

        let ingestion_time = table
            .columns
            .iter()
            .find(|c| c.name == "ingestion_time")
            .unwrap();
        assert_eq!(ingestion_time.data_latency.as_ref().unwrap().to_days(), 0);

        let amount = table.columns.iter().find(|c| c.name == "amount").unwrap();
        assert!(amount.data_latency.is_none());
    }
}

#[derive(Debug, Error)]
pub enum SourcesError {
    #[error("Failed to load sources file: {path}\n{source}")]
    LoadError {
        path: std::path::PathBuf,
        source: anyhow::Error,
    },

    #[error("Failed to parse sources YAML: {0}")]
    ParseError(#[from] serde_yaml::Error),
}
