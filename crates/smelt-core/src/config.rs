use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::metadata::ModelMetadata;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to load configuration file: {path}\n{source}")]
    LoadError {
        path: PathBuf,
        source: anyhow::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Materialization {
    Table,
    View,
    /// Not materialized — inlined as a CTE into downstream models.
    Ephemeral,
    /// Backend-managed persistent view (e.g., PostgreSQL, Databricks).
    MaterializedView,
    /// Test model — not materialized, used for unit testing.
    Test,
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
            "ephemeral" => Ok(Materialization::Ephemeral),
            "materialized_view" => Ok(Materialization::MaterializedView),
            "test" => Ok(Materialization::Test),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid materialization type: {}. Must be 'table', 'view', 'ephemeral', 'materialized_view', or 'test'",
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
            Materialization::Ephemeral => serializer.serialize_str("ephemeral"),
            Materialization::MaterializedView => serializer.serialize_str("materialized_view"),
            Materialization::Test => serializer.serialize_str("test"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub name: String,
    pub version: u32,
    #[serde(default = "default_model_paths")]
    pub model_paths: Vec<String>,
    #[serde(default = "default_seed_paths")]
    pub seed_paths: Vec<String>,
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

fn default_seed_paths() -> Vec<String> {
    vec!["seeds".to_string()]
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
    /// Target to execute this model on (overrides CLI --target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Day of the week for weekly partition start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

/// Data latency for a column — how late data can arrive.
///
/// Parsed from SQL interval syntax (e.g., "3 days", "1 hour", "0 hours").
/// Stored as a number of seconds for precise comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataLatency {
    /// Latency in seconds (for comparison and arithmetic).
    pub seconds: u64,
    /// Original string representation (for display).
    pub display: String,
}

impl DataLatency {
    /// Parse a SQL interval string like "3 days", "1 hour", "0 hours", "2 weeks".
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let n: u64 = parts[0].parse().ok()?;
        let unit = if parts.len() > 1 {
            parts[1].to_lowercase()
        } else {
            return None;
        };

        let seconds = match unit.trim_end_matches('s') {
            "hour" => n * 3600,
            "day" => n * 86400,
            "week" => n * 7 * 86400,
            "month" => n * 30 * 86400, // Approximate
            "year" => n * 365 * 86400, // Approximate
            _ => return None,
        };

        Some(DataLatency {
            seconds,
            display: s.to_string(),
        })
    }

    /// Convert to days (rounded up).
    pub fn to_days(&self) -> u32 {
        self.seconds.div_ceil(86400) as u32
    }

    /// Zero latency.
    pub fn zero() -> Self {
        DataLatency {
            seconds: 0,
            display: "0 hours".to_string(),
        }
    }
}

impl<'de> Deserialize<'de> for DataLatency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        DataLatency::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid data_latency '{}': expected format like '3 days', '1 hour', '0 hours'",
                s
            ))
        })
    }
}

/// Granularity for incremental partition generation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Hour,
    Day,
    Week { week_start: Weekday },
    Month,
    Quarter,
    Year,
}

/// Safety overrides for incremental materialization checks.
///
/// Each flag allows a specific pattern that is normally rejected
/// because it can produce different results on partial vs full data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct IncrementalSafetyOverrides {
    #[serde(default)]
    pub allow_window_functions: bool,
    #[serde(default)]
    pub allow_having: bool,
    #[serde(default)]
    pub allow_limit: bool,
    #[serde(default)]
    pub allow_subqueries: bool,
    #[serde(default)]
    pub allow_nondeterministic: bool,
    #[serde(default)]
    pub allow_distinct: bool,
}

/// Strategy for incremental materialization.
///
/// Model authors declare *what* (unique_key, partition_column) and backends
/// decide *how* (which strategy to use) via `resolve_strategy()`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalStrategy {
    DeleteInsert,
    Merge,
    Append,
    InsertOverwrite,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct IncrementalConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Column in source data to filter on (for WHERE injection)
    pub event_time_column: String,
    /// Column in output to delete by (for DELETE+INSERT)
    pub partition_column: String,
    /// Partition granularity (hour, day, week, month, quarter, year)
    pub granularity: Granularity,
    /// Columns that uniquely identify a row (backend uses presence to choose strategy)
    #[serde(default)]
    pub unique_key: Vec<String>,
    /// Safety overrides for patterns that may diverge on partial data
    #[serde(default)]
    pub safety_overrides: IncrementalSafetyOverrides,
}

impl Config {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join("smelt.yml");
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| ConfigError::LoadError {
                path: config_path.clone(),
                source: e.into(),
            })?;

        serde_yaml::from_str(&content).map_err(|e| {
            ConfigError::LoadError {
                path: config_path,
                source: e.into(),
            }
            .into()
        })
    }

    /// Get materialization for a model
    ///
    /// **Precedence**: smelt.yml model config > default_materialization
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
        sql_metadata: Option<&ModelMetadata>,
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
    pub fn get_tags(&self, model_name: &str, metadata: Option<&ModelMetadata>) -> Vec<String> {
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

    /// Get the target for a model
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config > default_target (CLI --target)
    pub fn get_target(
        &self,
        model_name: &str,
        sql_metadata: Option<&ModelMetadata>,
        default_target: &str,
    ) -> String {
        // Check SQL metadata first
        if let Some(metadata) = sql_metadata {
            if let Some(ref target) = metadata.target {
                return target.clone();
            }
        }

        // Check smelt.yml model config
        if let Some(model_config) = self.models.get(model_name) {
            if let Some(ref target) = model_config.target {
                return target.clone();
            }
        }

        // Fall back to default (CLI --target)
        default_target.to_string()
    }

    /// Get incremental config with SQL metadata precedence
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config
    pub fn get_incremental_with_metadata<'a>(
        &'a self,
        model_name: &str,
        sql_metadata: Option<&'a ModelMetadata>,
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

    /// Validate model configuration for materialization constraints.
    ///
    /// Returns a list of (model_name, error_message) for hard errors
    /// and prints warnings to stderr for soft issues.
    pub fn validate_model_configs(
        &self,
        model_metadata: &HashMap<String, ModelMetadata>,
    ) -> Vec<(String, String)> {
        let mut errors = Vec::new();

        // Collect all model names and their effective materialization + config
        let mut all_models: HashMap<
            &str,
            (Materialization, Option<&IncrementalConfig>, Option<&str>),
        > = HashMap::new();

        // From smelt.yml
        for (name, model_config) in &self.models {
            let mat = model_config
                .materialization
                .clone()
                .unwrap_or_else(|| self.default_materialization.clone());
            all_models.insert(
                name.as_str(),
                (
                    mat,
                    model_config.incremental.as_ref(),
                    model_config.target.as_deref(),
                ),
            );
        }

        // Override with SQL metadata (higher precedence)
        for (name, metadata) in model_metadata {
            let entry = all_models
                .entry(name.as_str())
                .or_insert_with(|| (self.default_materialization.clone(), None, None));
            if let Some(mat) = &metadata.materialization {
                entry.0 = mat.clone();
            }
            if let Some(inc) = &metadata.incremental {
                entry.1 = Some(inc);
            }
            if let Some(target) = &metadata.target {
                entry.2 = Some(target.as_str());
            }
        }

        for (name, (mat, incremental, target)) in &all_models {
            match mat {
                Materialization::Ephemeral => {
                    if incremental.is_some() {
                        errors.push((
                            name.to_string(),
                            "Ephemeral models cannot have incremental configuration".to_string(),
                        ));
                    }
                    if target.is_some() {
                        errors.push((
                            name.to_string(),
                            "Ephemeral models cannot have a target override".to_string(),
                        ));
                    }
                }
                Materialization::View => {
                    if let Some(inc) = incremental {
                        if inc.enabled {
                            eprintln!(
                                "  Warning: model '{}' is a view but has incremental config — incremental only applies to tables",
                                name
                            );
                        }
                    }
                }
                Materialization::MaterializedView => {
                    if let Some(inc) = incremental {
                        if inc.enabled {
                            eprintln!(
                                "  Warning: model '{}' is a materialized view but has incremental config — materialized views are refreshed atomically",
                                name
                            );
                        }
                    }
                }
                Materialization::Test => {
                    if incremental.is_some() {
                        errors.push((
                            name.to_string(),
                            "Test models cannot have incremental configuration".to_string(),
                        ));
                    }
                    if target.is_some() {
                        errors.push((
                            name.to_string(),
                            "Test models cannot have a target override".to_string(),
                        ));
                    }
                }
                Materialization::Table => {} // All config is valid for tables
            }
        }

        errors
    }
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

    #[test]
    fn test_quarter_granularity_deserialization() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: quarter
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.granularity, Granularity::Quarter);
        assert!(config.enabled); // default_enabled = true
    }

    #[test]
    fn test_year_granularity_deserialization() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: year
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.granularity, Granularity::Year);
    }

    #[test]
    fn test_safety_overrides_default_when_absent() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.safety_overrides,
            IncrementalSafetyOverrides::default()
        );
        assert!(!config.safety_overrides.allow_window_functions);
    }

    #[test]
    fn test_unique_key_defaults_empty() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.unique_key.is_empty());
    }

    #[test]
    fn test_unique_key_deserialization() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
            unique_key:
              - id
              - source
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.unique_key, vec!["id", "source"]);
    }

    #[test]
    fn test_incremental_strategy_serialization() {
        let strategy = IncrementalStrategy::DeleteInsert;
        let json = serde_json::to_string(&strategy).unwrap();
        assert_eq!(json, r#""delete_insert""#);

        let strategy: IncrementalStrategy = serde_json::from_str(r#""merge""#).unwrap();
        assert_eq!(strategy, IncrementalStrategy::Merge);
    }

    #[test]
    fn test_data_latency_parse() {
        let l = DataLatency::parse("3 days").unwrap();
        assert_eq!(l.seconds, 3 * 86400);
        assert_eq!(l.to_days(), 3);

        let l = DataLatency::parse("1 hour").unwrap();
        assert_eq!(l.seconds, 3600);
        assert_eq!(l.to_days(), 1); // rounds up

        let l = DataLatency::parse("0 hours").unwrap();
        assert_eq!(l.seconds, 0);
        assert_eq!(l.to_days(), 0);

        let l = DataLatency::parse("2 weeks").unwrap();
        assert_eq!(l.seconds, 2 * 7 * 86400);
        assert_eq!(l.to_days(), 14);

        assert!(DataLatency::parse("invalid").is_none());
        assert!(DataLatency::parse("3").is_none()); // no unit
    }

    #[test]
    fn test_data_latency_deserialization() {
        let yaml = r#""3 days""#;
        let latency: DataLatency = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(latency.to_days(), 3);
    }

    #[test]
    fn test_model_config_target_field() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  model_a:
    target: spark_prod
  model_b:
    materialization: table
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.models.get("model_a").unwrap().target,
            Some("spark_prod".to_string())
        );
        assert_eq!(config.models.get("model_b").unwrap().target, None);
    }

    #[test]
    fn test_get_target_precedence() {
        let yaml = r#"
name: test
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
  spark_prod:
    type: spark
    connect_url: sc://host:15002
    schema: prod
models:
  model_with_config_target:
    target: spark_prod
  model_no_target:
    materialization: table
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        // No metadata, no config target → default
        assert_eq!(config.get_target("model_no_target", None, "dev"), "dev");

        // Config target set → config wins over default
        assert_eq!(
            config.get_target("model_with_config_target", None, "dev"),
            "spark_prod"
        );

        // Metadata target overrides config target
        let metadata = ModelMetadata {
            target: Some("dev".to_string()),
            ..Default::default()
        };
        assert_eq!(
            config.get_target("model_with_config_target", Some(&metadata), "dev"),
            "dev"
        );

        // Unknown model → default
        assert_eq!(config.get_target("unknown_model", None, "dev"), "dev");
    }

    #[test]
    fn test_ephemeral_deserialization() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  staging_users:
    materialization: ephemeral
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.models.get("staging_users").unwrap().materialization,
            Some(Materialization::Ephemeral)
        );
    }

    #[test]
    fn test_materialized_view_deserialization() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  cached_report:
    materialization: materialized_view
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.models.get("cached_report").unwrap().materialization,
            Some(Materialization::MaterializedView)
        );
    }

    #[test]
    fn test_validate_ephemeral_with_incremental_errors() {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: HashMap::new(),
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "my_model".to_string(),
            ModelMetadata {
                materialization: Some(Materialization::Ephemeral),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    event_time_column: "ts".to_string(),
                    partition_column: "dt".to_string(),
                    granularity: Granularity::Day,
                    unique_key: vec![],
                    safety_overrides: IncrementalSafetyOverrides::default(),
                }),
                ..Default::default()
            },
        );

        let errors = config.validate_model_configs(&metadata);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("incremental"));
    }

    #[test]
    fn test_validate_ephemeral_with_target_errors() {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: HashMap::new(),
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "my_model".to_string(),
            ModelMetadata {
                materialization: Some(Materialization::Ephemeral),
                target: Some("spark_prod".to_string()),
                ..Default::default()
            },
        );

        let errors = config.validate_model_configs(&metadata);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("target"));
    }

    #[test]
    fn test_validate_table_with_incremental_ok() {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: HashMap::new(),
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "my_model".to_string(),
            ModelMetadata {
                materialization: Some(Materialization::Table),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    event_time_column: "ts".to_string(),
                    partition_column: "dt".to_string(),
                    granularity: Granularity::Day,
                    unique_key: vec![],
                    safety_overrides: IncrementalSafetyOverrides::default(),
                }),
                ..Default::default()
            },
        );

        let errors = config.validate_model_configs(&metadata);
        assert!(errors.is_empty());
    }
}
