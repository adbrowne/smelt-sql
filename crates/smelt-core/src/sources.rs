//! Per-entity source YAML loading for smelt.
//!
//! Phase 6: sources are now discovered from the filesystem as standalone `.yml`
//! files (no sibling `.csv`). The old aggregate `sources.yml` at the project
//! root is no longer supported — its presence is a hard error.
//!
//! References: docs/specs/sources.md §"Source YAML shape" and §"Filesystem layout"

use crate::config::TimeseriesConfig;
use crate::discovery::ModelDiscovery;
use crate::resolver::WorkspaceLoadError;
use serde::Deserialize;
use smelt_types::{parse_type, DataType};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The `name:` override in a per-entity source YAML.
///
/// Per `docs/specs/sources.md` §"Target-aware `name:` override":
/// - `Literal` — a single `<schema>.<table>` string applied to every target.
/// - `PerTarget` — a map from target name to `<schema>.<table>`, so different
///   targets can resolve to different external schemas/tables. Targets absent
///   from the map fall back to the default mapping.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SourceNameOverride {
    /// A single `<schema>.<table>` literal applied to all targets.
    Literal(String),
    /// Per-target map: `{ dev: raw_dev.users, prod: raw.users }`.
    PerTarget(BTreeMap<String, String>),
}

/// Information about a single source discovered from a per-entity `.yml` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// Absolute path to the `.yml` file on disk.
    pub path: PathBuf,
    /// Address segments stripped from scan-root to stem.
    ///
    /// For `models/sources/raw/users.yml` under `paths: ["models"]` this is
    /// `["sources", "raw", "users"]`.
    pub address_segments: Vec<String>,
    /// Declared columns (required on sources).
    pub columns: Vec<SourceColumn>,
    /// Optional free-text description.
    pub description: Option<String>,
    /// Optional `name:` override for the database table name (literal or per-target map).
    /// When `None`, the default mapping `<target_schema>.<address_segments.join("_")>`
    /// is used.
    pub name_override: Option<SourceNameOverride>,
    /// Tags declared in the source YAML (`tags:` key). Used by wide-reflection
    /// accessors `smelt.sources.with_tag` and `smelt.sources.all`.
    pub tags: Vec<String>,
    /// Optional time dimension declaration. When present, the source is a
    /// pushdown target for incremental models that reference it.
    pub timeseries: Option<TimeseriesConfig>,
}

impl SourceInfo {
    /// Returns the fully-qualified database name for this source.
    ///
    /// For `Literal` overrides, returns the literal verbatim (any target).
    /// For `PerTarget` overrides or `None`, returns the default mapping
    /// `<target_schema>.<address_segments.join("_")>`.
    ///
    /// For per-target resolution with the active target name, use
    /// `db_name_for_target` (introduced in P2).
    pub fn db_name(&self, target_schema: &str) -> String {
        match &self.name_override {
            Some(SourceNameOverride::Literal(s)) => s.clone(),
            Some(SourceNameOverride::PerTarget(_)) | None => {
                format!("{}.{}", target_schema, self.address_segments.join("_"))
            }
        }
    }
}

/// A single column declared in a source YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColumn {
    pub name: String,
    pub data_type: DataType,
    /// Whether NULL values are permitted (default: `true`).
    pub nullable: bool,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from parsing or discovering source YAML files.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML parse error in {path}: {message}")]
    YamlParse { path: PathBuf, message: String },

    #[error("Sources must declare `columns:`")]
    MissingColumns,

    #[error("`materialization:` is not allowed on a source — use a seed sidecar for seeds")]
    MaterializationForbidden,

    #[error("Unknown type '{type_str}' in source column '{column}'")]
    UnknownType { type_str: String, column: String },

    #[error("`name:` must be in `<schema>.<table>` format, got '{0}'")]
    InvalidNameOverride(String),
}

// ---------------------------------------------------------------------------
// Internal deserialization helpers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSourceYaml {
    /// Optional description.
    #[serde(default)]
    description: Option<String>,

    /// Columns declaration (required on sources).
    #[serde(default)]
    columns: Option<Vec<RawColumn>>,

    /// Optional `name:` override — literal `<schema>.<table>` or per-target map.
    #[serde(default)]
    name: Option<SourceNameOverride>,

    /// Presence of this key is a hard error on source YAMLs.
    #[serde(default)]
    materialization: Option<serde_yaml::Value>,

    /// Optional tags for filtering via `smelt.sources.with_tag`.
    #[serde(default)]
    tags: Vec<String>,

    /// Optional time dimension declaration. When present, the source is a
    /// pushdown target for incremental models.
    #[serde(default)]
    timeseries: Option<TimeseriesConfig>,
}

#[derive(Deserialize)]
struct RawColumn {
    name: String,
    #[serde(rename = "type", default)]
    type_str: Option<String>,
    #[serde(default = "default_nullable")]
    nullable: bool,
    #[serde(default)]
    description: Option<String>,
}

fn default_nullable() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a single source YAML file from disk.
///
/// The returned `SourceInfo` has `address_segments` computed from the file
/// stem only (no scan-root stripping). Callers that need the full address
/// should use [`discover_source_infos`] instead.
///
/// # Errors
/// - `SourceError::Io` — file cannot be read.
/// - `SourceError::YamlParse` — YAML is malformed.
/// - `SourceError::MissingColumns` — `columns:` key is absent.
/// - `SourceError::MaterializationForbidden` — `materialization:` key present.
/// - `SourceError::UnknownType` — a column type string is unrecognised.
/// - `SourceError::InvalidNameOverride` — `name:` is not `<schema>.<table>`.
pub fn parse_source_yaml(path: &Path) -> Result<SourceInfo, SourceError> {
    let text = std::fs::read_to_string(path).map_err(|e| SourceError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let raw: RawSourceYaml = serde_yaml::from_str(&text).map_err(|e| SourceError::YamlParse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    // `materialization:` is forbidden on sources.
    if raw.materialization.is_some() {
        return Err(SourceError::MaterializationForbidden);
    }

    // `columns:` is required.
    let raw_cols = raw.columns.ok_or(SourceError::MissingColumns)?;

    // Validate `name:` format — each value must be `<schema>.<table>`.
    if let Some(ref name_override) = raw.name {
        match name_override {
            SourceNameOverride::Literal(s) => {
                if !s.contains('.') || s.starts_with('.') || s.ends_with('.') {
                    return Err(SourceError::InvalidNameOverride(s.clone()));
                }
            }
            SourceNameOverride::PerTarget(map) => {
                for value in map.values() {
                    if !value.contains('.') || value.starts_with('.') || value.ends_with('.') {
                        return Err(SourceError::InvalidNameOverride(value.clone()));
                    }
                }
            }
        }
    }

    // Parse columns.
    let columns = raw_cols
        .into_iter()
        .map(|c| {
            let data_type = match &c.type_str {
                None => DataType::Text, // default when no type given
                Some(ts) => parse_type(ts).map_err(|_| SourceError::UnknownType {
                    type_str: ts.clone(),
                    column: c.name.clone(),
                })?,
            };
            Ok(SourceColumn {
                name: c.name,
                data_type,
                nullable: c.nullable,
                description: c.description,
            })
        })
        .collect::<Result<Vec<_>, SourceError>>()?;

    // Derive address_segments from the file stem (single segment for direct calls).
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    Ok(SourceInfo {
        path: path.to_path_buf(),
        address_segments: vec![stem],
        columns,
        description: raw.description,
        name_override: raw.name,
        tags: raw.tags,
        timeseries: raw.timeseries,
    })
}

/// Walk the project root and return every candidate source `.yml`/`.yaml` file —
/// a standalone YAML with no same-stem `.csv` sibling.
///
/// Discovery is project-wide (D-01: `paths:` is a strip-list, not a scan gate).
/// The `paths` parameter is retained for the `address_segments` derivation in
/// callers but is no longer used as a scan gate here.
///
/// This is the single place the source/sidecar disambiguation lives, shared by
/// [`discover_source_infos`] and [`discover_source_errors`].
fn candidate_source_yaml_files(project_dir: &Path, paths: &[String]) -> Vec<(PathBuf, PathBuf)> {
    use crate::discovery::project_root_files_by_dir;
    use crate::resolver::{classify, EntityKind};

    let mut candidates = Vec::new();

    for (_, files) in project_root_files_by_dir(project_dir) {
        for file_path in &files {
            if matches!(classify(file_path, None, &files), Some(EntityKind::Source)) {
                // root_dir kept as project_dir for legacy API compatibility;
                // address computation uses the full project-root-relative path.
                candidates.push((project_dir.to_path_buf(), file_path.clone()));
            }
        }
    }

    // Suppress unused-variable warning: paths is used by callers for address stripping
    let _ = paths;
    candidates
}

/// Walk all `paths` under `project_root`, find standalone `.yml` files (those
/// without a same-stem `.csv` sibling), parse them as sources, and return the
/// results. Files that fail to parse are silently skipped (errors surfaced via
/// [`discover_source_errors`] / diagnostics by the Salsa layer).
///
/// The `address_segments` field is populated with the full scan-root-stripped
/// path tuple (e.g. `["sources", "raw", "users"]` for
/// `models/sources/raw/users.yml` under `paths: ["models"]`).
pub fn discover_source_infos(project_dir: &Path, paths: &[String]) -> Vec<SourceInfo> {
    let mut sources = Vec::new();

    for (_root_dir, file_path) in candidate_source_yaml_files(project_dir, paths) {
        // Parse the source YAML.
        let mut info = match parse_source_yaml(&file_path) {
            Ok(i) => i,
            Err(_) => continue, // surfaced via discover_source_errors
        };

        // Address via the D-01 strip-list rule (shared with models/seeds).
        info.address_segments =
            ModelDiscovery::compute_address_segments(&file_path, project_dir, paths);

        sources.push(info);
    }

    sources.sort_by(|a, b| a.address_segments.cmp(&b.address_segments));
    sources
}

/// Walk the same candidate source `.yml` files as [`discover_source_infos`] but
/// return only the ones that **fail** to parse, each paired with its
/// [`SourceError`]. The Salsa source-diagnostics producer maps these into
/// `MalformedSource` / `SourceTypeError` diagnostics so a malformed per-entity
/// source is visible to the analyzer (and the build gate) instead of being
/// silently dropped by discovery.
///
/// Results are sorted by path for deterministic diagnostic ordering.
pub fn discover_source_errors(project_dir: &Path, paths: &[String]) -> Vec<(PathBuf, SourceError)> {
    let mut errors: Vec<(PathBuf, SourceError)> = Vec::new();

    for (_root_dir, file_path) in candidate_source_yaml_files(project_dir, paths) {
        if let Err(e) = parse_source_yaml(&file_path) {
            errors.push((file_path, e));
        }
    }

    errors.sort_by(|a, b| a.0.cmp(&b.0));
    errors
}

/// Check whether an aggregate `sources.yml` or `sources.yaml` exists at the
/// project root and return an error if so.
///
/// This is the migration guard: any project that still has an aggregate
/// sources file gets a clear error directing them to the per-entity layout.
pub fn check_aggregate_sources_yml(project_root: &Path) -> Result<(), WorkspaceLoadError> {
    let yml = project_root.join("sources.yml");
    let yaml = project_root.join("sources.yaml");

    if yml.exists() {
        return Err(WorkspaceLoadError::AggregateSourcesYmlNotSupported { path: yml });
    }
    if yaml.exists() {
        return Err(WorkspaceLoadError::AggregateSourcesYmlNotSupported { path: yaml });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy types — kept for backward compatibility with the aggregate loader.
// These are still used by the CLI compiler, temporal, backfill, etc.
// until they are also migrated in later phases.
// ---------------------------------------------------------------------------

use crate::config::DataLatency;
use std::collections::HashMap;

/// Sources configuration from the legacy aggregate sources.yml.
/// Still used by CLI code paths that haven't been migrated to per-entity YAMLs.
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
        struct RawColumn2 {
            name: String,
            #[serde(default, rename = "type")]
            type_str: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            data_latency: Option<DataLatency>,
        }

        let raw = RawColumn2::deserialize(deserializer)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 1 / BUG-072: a source YAML with a `timeseries:` block parses to
    /// `SourceInfo.timeseries == Some(TimeseriesConfig { ... })`.
    #[test]
    fn source_yaml_timeseries_block_parses() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.yml");
        std::fs::write(
            &path,
            r#"
description: Raw events
columns:
  - name: event_id
    type: BIGINT
    nullable: false
  - name: event_ts
    type: TIMESTAMP
    nullable: false
  - name: event_date
    type: DATE
    nullable: false
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
"#,
        )
        .unwrap();

        let info = parse_source_yaml(&path).expect("should parse");
        let ts = info.timeseries.expect("timeseries should be Some");
        assert_eq!(ts.event_time_column, "event_ts");
        assert_eq!(ts.partition_column, "event_date");
        assert_eq!(
            ts.granularity,
            crate::config::Granularity::Day,
            "granularity should be Day"
        );
    }

    /// Phase 1 / BUG-072: a source YAML with an unrecognised key (e.g. a typo
    /// `timseries:`) must return an error rather than silently discarding it.
    #[test]
    fn source_yaml_unknown_key_is_loud() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.yml");
        std::fs::write(
            &path,
            r#"
description: Raw events
columns:
  - name: event_id
    type: BIGINT
timseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
"#,
        )
        .unwrap();

        let result = parse_source_yaml(&path);
        assert!(
            result.is_err(),
            "unknown key `timseries:` should be a parse error, not silently dropped"
        );
        match result {
            Err(SourceError::YamlParse { message, .. }) => {
                assert!(
                    message.to_lowercase().contains("timseries")
                        || message.to_lowercase().contains("unknown"),
                    "error message should mention the unknown key, got: {message}"
                );
            }
            Err(other) => panic!("expected YamlParse error, got: {other:?}"),
            Ok(_) => unreachable!(),
        }
    }

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

    /// A standalone .yml in `billing/raw/events.yml` (not in `paths: ["models"]`)
    /// must be discovered project-wide after D-01 universal walk.
    /// Spec: architecture.md §"Resolution" — project-wide discovery for sources.
    #[test]
    fn source_discovered_project_wide() {
        let dir = tempfile::TempDir::new().unwrap();
        // paths: ["models"] — billing/ is NOT in paths
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        let raw_dir = dir.path().join("billing").join("raw");
        std::fs::create_dir_all(&raw_dir).unwrap();
        std::fs::write(
            raw_dir.join("events.yml"),
            "description: raw events\ncolumns:\n  - name: id\n    type: Integer\n",
        )
        .unwrap();

        let sources = discover_source_infos(dir.path(), &["models".to_string()]);
        assert!(
            sources
                .iter()
                .any(|s| s.address_segments.last() == Some(&"events".to_string())),
            "events.yml outside models/ must be discovered project-wide; got: {:?}",
            sources
                .iter()
                .map(|s| &s.address_segments)
                .collect::<Vec<_>>()
        );
        let events = sources
            .iter()
            .find(|s| s.address_segments.last() == Some(&"events".to_string()))
            .unwrap();
        assert_eq!(
            events.address_segments,
            vec!["billing", "raw", "events"],
            "address must include full path since billing/ is not a paths prefix"
        );
    }
}
