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
    /// Cumulative aggregate — stateful merge into one row per GROUP BY key.
    /// Unique key, per-column aggregator, and cross-partition combiner are
    /// derived from the SELECT (see `docs/specs/cumulative_aggregate.md`).
    CumulativeAggregate,
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
            "cumulative_aggregate" => Ok(Materialization::CumulativeAggregate),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid materialization type: {}. Must be 'table', 'view', 'ephemeral', 'materialized_view', 'test', or 'cumulative_aggregate'",
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
            Materialization::CumulativeAggregate => {
                serializer.serialize_str("cumulative_aggregate")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub name: String,
    /// Schema version of the smelt.yml file format. Optional — defaults to 1.
    /// Made optional to remove a confusing trip-hazard where new users
    /// instinctively wrote a semver string (`version: "0.1.0"`, mirroring
    /// pyproject.toml) and got a parse error. The field is decorative today
    /// and only printed in run logs. (iter-4 issue #1.)
    #[serde(default = "default_config_version")]
    pub version: u32,
    /// Workspace-relative directories scanned for project files (`.sql`, `.py`, `.csv`, `.yml`).
    /// Replaces the legacy `model_paths` + `seed_paths` split — kind is
    /// determined by file format/content (`architecture.md` §"Resolution"),
    /// not by which directory the file lives in.
    #[serde(default = "default_paths")]
    pub paths: Vec<String>,
    pub targets: HashMap<String, Target>,
    #[serde(default = "default_materialization")]
    pub default_materialization: Materialization,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
    /// Path to Python interpreter (overridden by SMELT_PYTHON env var)
    #[serde(default)]
    pub python: Option<String>,
    /// Default active build target (key into `targets`). Both the CLI and the LSP
    /// use this as the effective target when no `--target` override is supplied.
    /// Absent when no default is configured — resolution falls back to base-only
    /// loader dispatch (no overlay files applied).
    #[serde(default)]
    pub target: Option<String>,
}

fn default_config_version() -> u32 {
    1
}

fn default_paths() -> Vec<String> {
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
    /// Base directory for file-based output (e.g., Spark warehouse for Parquet files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warehouse: Option<String>,
    /// Table format for Spark targets: "delta" (default) or "parquet".
    /// Ignored for DuckDB targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<TableFormat>,
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

    /// Get the effective table format for this target.
    ///
    /// Returns `None` for DuckDB targets (format is not applicable).
    /// For Spark targets, defaults to `Delta` if not specified.
    pub fn table_format(&self) -> Option<TableFormat> {
        match self.backend_type() {
            BackendType::DuckDB => None,
            BackendType::Spark => Some(self.format.unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    DuckDB,
    Spark,
}

/// Table format for Spark targets.
///
/// DuckDB targets ignore this field. Spark targets use it to determine
/// schema evolution capabilities (e.g., Delta supports column mapping
/// while Parquet does not).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum TableFormat {
    #[default]
    Delta,
    Parquet,
}

impl<'de> Deserialize<'de> for TableFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "delta" => Ok(TableFormat::Delta),
            "parquet" => Ok(TableFormat::Parquet),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid table format: {}. Must be 'delta' or 'parquet'",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub materialization: Option<Materialization>,
    #[serde(default)]
    pub timeseries: Option<TimeseriesConfig>,
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
///
/// A closed enum of supported time-unit boundaries. `week_start` for weekly
/// partitions lives in `TimeseriesConfig.week_start`, not in this variant.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Hour,
    Day,
    Week,
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
///
/// UPSERT (`MERGE`) is **not** an incremental strategy — it is the physical
/// primitive used by the `cumulative_aggregate` materialization
/// (`docs/specs/cumulative_aggregate.md`), which is a separate sibling rule
/// with a different equivalence contract. `Backend::merge_into` remains on
/// the backend trait for that caller; it is not reachable from this enum.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalStrategy {
    DeleteInsert,
    Append,
    InsertOverwrite,
}

fn default_enabled() -> bool {
    true
}

/// Time-dimension declaration for a model or source output.
///
/// Factored out of `IncrementalConfig` so that views, non-incremental tables,
/// and external sources can declare a time dimension without opting into
/// incremental execution. `incremental:` consumes this block; any model
/// declaring `incremental:` must also declare `timeseries:`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimeseriesConfig {
    /// Source-of-truth time column (timestamp or date).
    pub event_time_column: String,
    /// Column the engine prunes on (date or integer).
    pub partition_column: String,
    /// Partition granularity.
    pub granularity: Granularity,
    /// Day of week for weekly partitions (only valid when `granularity` is `week`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week_start: Option<Weekday>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Columns that uniquely identify a row (backend uses presence to choose strategy)
    #[serde(default)]
    pub unique_key: Vec<String>,
    /// Safety overrides for patterns that may diverge on partial data
    #[serde(default)]
    pub safety_overrides: IncrementalSafetyOverrides,
}

/// Parse the `unstable_schema:` flag from the text of a `smelt.yml` file.
///
/// Returns `true` when the text contains `unstable_schema: true`.
/// Returns `false` when the key is absent or set to anything else.
/// Pure function — takes the text rather than a path.
pub fn parse_unstable_schema_flag(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("unstable_schema:") {
            return rest.trim() == "true";
        }
    }
    false
}

/// Parse the distinct `target_type` values from the `targets:` block of a
/// `smelt.yml` file. Pure function — takes the text rather than a path.
///
/// Returns the de-duplicated, lower-cased target types in sorted order, or
/// `None` if the YAML cannot be parsed.
pub fn parse_active_backends(text: &str) -> Option<Vec<String>> {
    if text.is_empty() {
        return None;
    }
    let config = serde_yaml::from_str::<Config>(text).ok()?;
    let mut backends: Vec<String> = config
        .targets
        .values()
        .map(|t| t.target_type.to_ascii_lowercase())
        .collect();
    backends.sort();
    backends.dedup();
    Some(backends)
}

impl Config {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join("smelt.yml");
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| ConfigError::LoadError {
                path: config_path.clone(),
                source: e.into(),
            })?;

        let (config, warnings) =
            Self::parse_with_warnings(&content).map_err(|e| ConfigError::LoadError {
                path: config_path,
                source: e.into(),
            })?;
        for w in &warnings {
            eprintln!("{}", w);
        }
        Ok(config)
    }

    /// Parse `smelt.yml` text into a `Config` plus any warnings about
    /// unknown / legacy top-level keys.
    ///
    /// Pure function — does not touch the filesystem and emits no side
    /// effects. Callers that want the warnings on stderr (`Config::load`)
    /// print them themselves.
    ///
    /// Recognises the legacy `model_paths` and `seed_paths` keys (replaced
    /// by the unified `paths:` list) and emits a warning naming them. The
    /// returned `Config.paths` is the default (`["models"]`) — legacy keys
    /// are silently ignored beyond the warning, per `smelt_yml.md`
    /// §"Unknown keys".
    pub fn parse_with_warnings(text: &str) -> Result<(Self, Vec<String>), serde_yaml::Error> {
        let config: Config = serde_yaml::from_str(text)?;
        let mut warnings = Vec::new();
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) {
            if let Some(map) = value.as_mapping() {
                // Emit targeted warnings for legacy keys (kept distinct from the generic pass
                // so callers see the actionable migration hint, not a generic "unknown key").
                for legacy in ["model_paths", "seed_paths"] {
                    if map.contains_key(serde_yaml::Value::String(legacy.to_string())) {
                        warnings.push(format!(
                            "warning: smelt.yml: ignoring legacy key `{}`. Use `paths:` instead — the single scan list (smelt_yml.md §Top-level keys).",
                            legacy
                        ));
                    }
                }

                // Generic unknown-key pass: warn for any top-level key not in the allow-list.
                // `model_paths`/`seed_paths` are included to suppress duplicate warnings
                // (they already got the targeted message above).
                // `unstable_schema` is consumed by `parse_unstable_schema_flag` and is not
                // a `Config` struct field — allow-list it to avoid false positives.
                const KNOWN_KEYS: &[&str] = &[
                    "name",
                    "version",
                    "paths",
                    "targets",
                    "default_materialization",
                    "models",
                    "python",
                    "model_paths",
                    "seed_paths",
                    "unstable_schema",
                ];
                for (key, _) in map {
                    if let Some(key_str) = key.as_str() {
                        if !KNOWN_KEYS.contains(&key_str) {
                            warnings.push(format!(
                                "warning: smelt.yml: unknown top-level key `{}` (ignored). See smelt_yml.md §Top-level keys.",
                                key_str
                            ));
                        }
                    }
                }
            }
        }
        Ok((config, warnings))
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

    /// Get timeseries config for a model if set
    ///
    /// **Precedence**: smelt.yml only (for now)
    pub fn get_timeseries(&self, model_name: &str) -> Option<&TimeseriesConfig> {
        self.models
            .get(model_name)
            .and_then(|m| m.timeseries.as_ref())
    }

    /// Get timeseries config with SQL metadata precedence
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config
    pub fn get_timeseries_with_metadata<'a>(
        &'a self,
        model_name: &str,
        sql_metadata: Option<&'a ModelMetadata>,
    ) -> Option<&'a TimeseriesConfig> {
        // Check SQL metadata first
        if let Some(metadata) = sql_metadata {
            if let Some(ref ts) = metadata.timeseries {
                return Some(ts);
            }
        }
        // Fall back to smelt.yml
        self.get_timeseries(model_name)
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
                Materialization::CumulativeAggregate => {
                    // `cumulative_aggregate` forbids `incremental:` — the two are
                    // different rules with different equivalence contracts.
                    // The `timeseries:` forbid is enforced in `validate_timeseries`
                    // (where the block is reachable).
                    if incremental.is_some() {
                        errors.push((
                            name.to_string(),
                            "CumulativeForbidsIncremental: cumulative_aggregate models cannot \
                             carry an `incremental:` block — they are sibling materializations \
                             with different equivalence contracts (see docs/specs/cumulative_aggregate.md)"
                                .to_string(),
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

    /// iter-4 issue #1: a smelt.yml without a `version` field must parse
    /// (defaulting to 1) so new users don't trip over a required field that
    /// is decorative today and only appears in run logs.
    #[test]
    fn config_version_defaults_to_one_when_omitted() {
        let yaml = r#"
name: test_project
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("config without version must parse");
        assert_eq!(config.version, 1);
    }

    /// A semver-style string in `version` (the natural mistake — mirrors
    /// pyproject.toml) must still produce a parse error rather than
    /// silently coercing. The error is the user-visible signal.
    #[test]
    fn config_version_rejects_semver_string() {
        let yaml = r#"
name: test_project
version: "0.1.0"
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;
        serde_yaml::from_str::<Config>(yaml)
            .expect_err("semver-string version must be rejected (use integer)");
    }

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
    fn test_materialization_cumulative_aggregate_parses() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  device_user_edges:
    materialization: cumulative_aggregate
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config
                .models
                .get("device_user_edges")
                .unwrap()
                .materialization,
            Some(Materialization::CumulativeAggregate)
        );
    }

    /// Cumulative aggregate models cannot carry an incremental: block. The
    /// validator emits a CumulativeForbidsIncremental-flavored error in
    /// the errors vector.
    #[test]
    fn test_validate_cumulative_aggregate_forbids_incremental() {
        use crate::metadata::ModelMetadata;

        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  bad_model:
    materialization: cumulative_aggregate
    incremental:
      enabled: true
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let errors = config.validate_model_configs(&HashMap::<String, ModelMetadata>::new());
        assert!(
            errors
                .iter()
                .any(|(name, msg)| name == "bad_model"
                    && msg.contains("CumulativeForbidsIncremental")),
            "Expected CumulativeForbidsIncremental error, got: {:?}",
            errors
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
        let config: TimeseriesConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.granularity, Granularity::Quarter);
    }

    #[test]
    fn test_year_granularity_deserialization() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: year
        "#;
        let config: TimeseriesConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.granularity, Granularity::Year);
    }

    #[test]
    fn test_timeseries_config_rejects_unknown_key() {
        // BUG-025: previously unknown sub-keys were silently accepted/dropped;
        // with deny_unknown_fields they must return a serde Err.
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
            partion_column: dt
        "#;
        let result: Result<TimeseriesConfig, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "typo'd sub-key should be rejected, not silently dropped"
        );
    }

    #[test]
    fn test_safety_overrides_default_when_absent() {
        let yaml = r#"
            enabled: true
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
            enabled: true
        "#;
        let config: IncrementalConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.unique_key.is_empty());
    }

    #[test]
    fn test_unique_key_deserialization() {
        let yaml = r#"
            enabled: true
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

        let strategy: IncrementalStrategy = serde_json::from_str(r#""append""#).unwrap();
        assert_eq!(strategy, IncrementalStrategy::Append);
    }

    /// `merge` is no longer an incremental strategy — UPSERT is the physical
    /// primitive used by the `cumulative_aggregate` materialization, not a
    /// knob on `incremental:`. Deserialising it must fail.
    #[test]
    fn test_incremental_strategy_no_merge_variant() {
        let result: Result<IncrementalStrategy, _> = serde_json::from_str(r#""merge""#);
        assert!(
            result.is_err(),
            "`merge` must not deserialise as an IncrementalStrategy — it is the \
             physical primitive of `materialization: cumulative_aggregate`"
        );
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
            paths: vec!["models".to_string()],
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
            paths: vec!["models".to_string()],
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
    fn test_table_format_deserialization() {
        // Spark target with explicit delta format
        let yaml = r#"
name: test_project
version: 1
targets:
  spark_dev:
    type: spark
    connect_url: sc://host:15002
    schema: dev
    format: delta
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("spark_dev").unwrap();
        assert_eq!(target.format, Some(TableFormat::Delta));
        assert_eq!(target.table_format(), Some(TableFormat::Delta));
    }

    #[test]
    fn test_table_format_parquet() {
        let yaml = r#"
name: test_project
version: 1
targets:
  spark_parquet:
    type: spark
    connect_url: sc://host:15002
    schema: dev
    format: parquet
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("spark_parquet").unwrap();
        assert_eq!(target.format, Some(TableFormat::Parquet));
        assert_eq!(target.table_format(), Some(TableFormat::Parquet));
    }

    #[test]
    fn test_table_format_defaults_to_delta_for_spark() {
        let yaml = r#"
name: test_project
version: 1
targets:
  spark_default:
    type: spark
    connect_url: sc://host:15002
    schema: dev
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("spark_default").unwrap();
        assert_eq!(target.format, None);
        // table_format() defaults to Delta for Spark
        assert_eq!(target.table_format(), Some(TableFormat::Delta));
    }

    #[test]
    fn test_table_format_none_for_duckdb() {
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
        let target = config.targets.get("dev").unwrap();
        assert_eq!(target.table_format(), None);
    }

    #[test]
    fn test_table_format_invalid_rejected() {
        let yaml = r#"
name: test_project
version: 1
targets:
  bad:
    type: spark
    connect_url: sc://host:15002
    schema: dev
    format: iceberg
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid table format"), "Error was: {}", err);
    }

    #[test]
    fn test_validate_table_with_incremental_ok() {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
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
                    unique_key: vec![],
                    safety_overrides: IncrementalSafetyOverrides::default(),
                }),
                ..Default::default()
            },
        );

        let errors = config.validate_model_configs(&metadata);
        assert!(errors.is_empty());
    }

    /// BUG-056: `event_time_column`/`partition_column`/`granularity` are fields
    /// on `timeseries:`, not `incremental:`. Because `IncrementalConfig` uses
    /// `deny_unknown_fields`, putting them under `incremental:` must fail at
    /// parse time rather than silently being dropped.
    #[test]
    fn incremental_config_rejects_timeseries_fields() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: ts
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "event_time_column under incremental: must fail — belongs under timeseries:"
        );
    }

    /// BUG-056 regression: correct format has `timeseries:` and `incremental:`
    /// as sibling keys on the model config, not nested.
    #[test]
    fn timeseries_and_incremental_are_sibling_keys() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  daily_revenue:
    materialization: table
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
    incremental:
      enabled: true
"#;
        let config: Config =
            serde_yaml::from_str(yaml).expect("timeseries + incremental as siblings must parse");
        let model = config.models.get("daily_revenue").unwrap();
        let ts = model.timeseries.as_ref().unwrap();
        assert_eq!(ts.event_time_column, "transaction_timestamp");
        assert_eq!(ts.partition_column, "revenue_date");
        assert_eq!(ts.granularity, Granularity::Day);
        let inc = model.incremental.as_ref().unwrap();
        assert!(inc.enabled);
    }

    /// `paths:` defaults to `["models"]` when omitted (`smelt_yml.md`
    /// Surface §"Top-level keys" / Semantics §5).
    #[test]
    fn paths_defaults_to_models() {
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
        assert_eq!(config.paths, vec!["models".to_string()]);
    }

    /// BUG-060: a typo'd top-level key emits exactly one warning naming that key.
    /// Parsing still succeeds; the unknown key is silently ignored (not an error).
    #[test]
    fn unknown_top_level_key_warns() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
default_matrialization: table
"#;
        let (config, warnings) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.name, "test_project");
        assert_eq!(
            warnings.len(),
            1,
            "expected exactly one unknown-key warning, got: {:?}",
            warnings
        );
        assert!(
            warnings[0].contains("default_matrialization"),
            "warning must name the offending key: {}",
            warnings[0]
        );
    }

    /// BUG-060: a fully-valid config (all known keys + unstable_schema) produces
    /// zero generic unknown-key warnings.
    #[test]
    fn valid_config_with_all_known_keys_emits_no_generic_warnings() {
        let yaml = r#"
name: test_project
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
default_materialization: table
models: {}
python: ~
unstable_schema: true
"#;
        let (_config, warnings) = Config::parse_with_warnings(yaml).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected for a fully-valid config, got: {:?}",
            warnings
        );
    }

    /// BUG-060: legacy model_paths produces only the targeted legacy warning,
    /// not an additional generic "unknown key" warning.
    #[test]
    fn legacy_path_key_does_not_also_get_generic_unknown_key_warning() {
        let yaml = r#"
name: test_project
version: 1
model_paths:
  - models
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;
        let (_config, warnings) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(
            warnings.len(),
            1,
            "legacy key must produce only the targeted legacy warning, not a duplicate generic one: {:?}",
            warnings
        );
        assert!(
            warnings[0].contains("model_paths"),
            "warning must name the legacy key: {}",
            warnings[0]
        );
    }

    /// `paths: [...]` round-trips through (de)serialization unchanged.
    /// Order is preserved.
    #[test]
    fn paths_round_trips() {
        let yaml = r#"
name: test_project
version: 1
paths:
  - models
  - fixtures
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.paths,
            vec!["models".to_string(), "fixtures".to_string()]
        );

        // Round-trip serialise → deserialise → expect same paths.
        let round_trip = serde_yaml::to_string(&config).unwrap();
        let config2: Config = serde_yaml::from_str(&round_trip).unwrap();
        assert_eq!(config2.paths, config.paths);
    }

    /// Legacy `model_paths` / `seed_paths` keys parse successfully (per the
    /// `smelt_yml.md` §"Unknown keys" rule) but the resulting `paths`
    /// field is the default. `parse_with_warnings` reports a warning
    /// naming each legacy key.
    #[test]
    fn legacy_path_keys_warn() {
        let yaml = r#"
name: test_project
version: 1
model_paths:
  - models
  - tests
seed_paths:
  - seeds
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#;
        let (config, warnings) = Config::parse_with_warnings(yaml).unwrap();

        // Legacy keys parse successfully — paths is the default.
        assert_eq!(config.paths, vec!["models".to_string()]);

        // Warnings are emitted for each legacy key.
        assert_eq!(warnings.len(), 2, "expected one warning per legacy key");
        let joined = warnings.join("\n");
        assert!(
            joined.contains("model_paths"),
            "warning text must name `model_paths`: {}",
            joined
        );
        assert!(
            joined.contains("seed_paths"),
            "warning text must name `seed_paths`: {}",
            joined
        );
        assert!(
            joined.to_lowercase().contains("paths"),
            "warning should refer to the replacement `paths:` key: {}",
            joined
        );
    }
}
