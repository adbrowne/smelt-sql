use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::warn;

use crate::metadata::ModelMetadata;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to load configuration file: {path}\n{source}")]
    LoadError {
        path: PathBuf,
        source: anyhow::Error,
    },
}

/// The refresh axis — the freshness-owner trichotomy for how a stored
/// model's output is recomputed across runs (`docs/specs/models.md`
/// §"Refresh axis").
///
/// `Full` is the default (no `grain:` needed): smelt recomputes everything
/// each run. `Incremental` means smelt keeps the table current by running
/// the derived maintenance plan each run — it additionally requires a
/// declared `grain:` (see [`Grain`]), since an incremental output needs a
/// declared row identity/shape. `MaterializedView` delegates freshness to
/// the backend's native incremental-view maintenance.
///
/// The former `batched`/`keyed` mode values are folded into `Incremental` +
/// `grain:`: `batched` ≡ `(Incremental, Grain::Partition)`, `keyed` ≡
/// `(Incremental, Grain::Key)`. The bare mode names no longer parse — see
/// the `Deserialize` impl's fix-it errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshStrategy {
    /// Rebuild from scratch on every run (default; no `grain:` required).
    Full,
    /// smelt keeps the stored table current by running the derived
    /// maintenance plan each run. Requires a sibling `grain:` declaration
    /// (see [`Grain`]) — the output's declared shape and row identity.
    Incremental,
    /// Engine-maintained materialized view: the backend keeps the output
    /// current with its own native incremental-view maintenance, not a
    /// smelt-driven refresh loop. Keyed output, like `Incremental` +
    /// `Grain::Key`; forbids `timeseries:`, `grain:`, and a `batched:`
    /// block. Requires the resolved backend's `supports_native_ivm`
    /// capability — otherwise a hard error, never a silent fallback
    /// (`docs/specs/materialized_view.md` §"No silent fallback").
    MaterializedView,
}

/// Fix-it text shared by every removed `refresh:` mode name — named as a
/// constant so every hard-cut error site (and its tests) agrees on the exact
/// replacement wording (`docs/specs/models.md` §"Refresh axis").
const REFRESH_INCREMENTAL_FIXIT: &str = "use `refresh: incremental` with the matching `grain:`";

impl<'de> Deserialize<'de> for RefreshStrategy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "full" => Ok(RefreshStrategy::Full),
            "incremental" => Ok(RefreshStrategy::Incremental),
            "materialized_view" => Ok(RefreshStrategy::MaterializedView),
            "batched" => Err(serde::de::Error::custom(format!(
                "Invalid refresh strategy: 'batched'. `refresh: batched` is now \
                 `refresh: incremental` with `grain: partition` — {} \
                 (see docs/specs/incremental_models.md)",
                REFRESH_INCREMENTAL_FIXIT
            ))),
            "keyed" => Err(serde::de::Error::custom(format!(
                "Invalid refresh strategy: 'keyed'. `refresh: keyed` is now \
                 `refresh: incremental` with `grain: key` — {} \
                 (see docs/specs/incremental_models.md)",
                REFRESH_INCREMENTAL_FIXIT
            ))),
            "cumulative" => Err(serde::de::Error::custom(format!(
                "Invalid refresh strategy: 'cumulative'. `refresh: cumulative` is now \
                 `refresh: incremental` with `grain: key` — {} \
                 (see docs/specs/incremental_models.md)",
                REFRESH_INCREMENTAL_FIXIT
            ))),
            "versioned" => Err(serde::de::Error::custom(
                "Invalid refresh strategy: 'versioned'. There is no versioned mode: SCD2 \
                 history is written as plain windowed SQL under `refresh: full` (or \
                 `refresh: materialized_view` where the engine has native IVM) \
                 (see docs/specs/incremental_models.md §Limitations)"
                    .to_string(),
            )),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid refresh strategy: {}. Must be 'full', 'incremental', or 'materialized_view'",
                s
            ))),
        }
    }
}

impl Serialize for RefreshStrategy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            RefreshStrategy::Full => serializer.serialize_str("full"),
            RefreshStrategy::Incremental => serializer.serialize_str("incremental"),
            RefreshStrategy::MaterializedView => serializer.serialize_str("materialized_view"),
        }
    }
}

/// The declared output shape and grain for an `refresh: incremental` model —
/// what a stored row *is* and how it is addressed. Required whenever
/// `refresh: incremental` is set; rejected (hard error) otherwise
/// (`docs/specs/models.md` §"Refresh axis").
///
/// `batched` (the former refresh mode) ≡ `Grain::Partition`; `keyed` ≡
/// `Grain::Key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    /// A stored row is one row of a complete, partition-addressed table.
    /// `unique_key` is optional (a within-partition dedup aid only, never
    /// key-addressing); `timeseries:` is required.
    Partition,
    /// A stored row is the end-state per key. `unique_key` is required
    /// (composite-valued); `timeseries:` is admitted only when key temporal
    /// locality is established (`docs/specs/incremental_shapes.md` §"Key temporal
    /// locality").
    Key,
    /// A stored row is the trajectory: one row per `(key, partition)`.
    /// `unique_key` and `timeseries:` are both required — the partition axis
    /// is half the grain.
    KeyPerPartition,
}

impl<'de> Deserialize<'de> for Grain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "partition" => Ok(Grain::Partition),
            "key" => Ok(Grain::Key),
            "key_per_partition" => Err(serde::de::Error::custom(
                "grain: key_per_partition cannot be declared — it is a derived-only label. \
                 It is derived from a `timeseries:` clock plus `partition_column ∈ unique_key`; \
                 the closest supported declared shape is `grain: key`.",
            )),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid grain: {}. Must be 'partition' or 'key'",
                s
            ))),
        }
    }
}

impl Serialize for Grain {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Grain::Partition => serializer.serialize_str("partition"),
            Grain::Key => serializer.serialize_str("key"),
            Grain::KeyPerPartition => serializer.serialize_str("key_per_partition"),
        }
    }
}

impl std::fmt::Display for Grain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Grain::Partition => "partition",
            Grain::Key => "key",
            Grain::KeyPerPartition => "key_per_partition",
        };
        write!(f, "{s}")
    }
}

/// Derive the `grain` label from the two declared-and-checked shape-defining
/// facts (`docs/specs/models.md` §"Refresh axis", §"The Relation Contract"):
/// the **clock** (a `timeseries:` block is present) and the **identity** (a
/// `unique_key:` is declared), plus whether the clock's `partition_column`
/// is itself a member of the identity.
///
/// Pure — no I/O, no SQL parsing. The four corners:
///
/// | clock | identity | `partition_column ∈ key` | Derived grain |
/// |---|---|---|---|
/// | yes | no | — | `Partition` |
/// | no | yes | — | `Key` |
/// | yes | yes | no | `Key` (time-partitioned) |
/// | yes | yes | yes | `KeyPerPartition` (the trajectory) |
///
/// Returns `None` when **neither** fact is present — there is nothing to
/// derive a shape from (`models.md` §"Constraint violations": "no
/// shape-defining fact declared"). Callers that have already established at
/// least one fact is present (e.g. by checking `clock || identity.is_some()`)
/// can `.expect()` the result; callers validating fresh frontmatter should
/// treat `None` as the "neither declared" hard-error case.
pub fn derive_grain(
    clock: bool,
    identity: Option<&[String]>,
    partition_col: Option<&str>,
) -> Option<Grain> {
    match (clock, identity) {
        (true, None) => Some(Grain::Partition),
        (false, Some(_)) => Some(Grain::Key),
        (true, Some(key)) => {
            let partition_in_key = partition_col
                .map(|p| key.iter().any(|k| k == p))
                .unwrap_or(false);
            Some(if partition_in_key {
                Grain::KeyPerPartition
            } else {
                Grain::Key
            })
        }
        (false, None) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Materialization {
    Table,
    View,
    /// Not materialized — inlined as a CTE into downstream models.
    Ephemeral,
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
            _ => Err(serde::de::Error::custom(format!(
                "Invalid materialization type: {}. Must be 'table', 'view', or 'ephemeral'. \
                 Note: 'test' has been removed — use `smelt.test` declarations instead. \
                 Note: 'cumulative_aggregate' has been removed — use `materialization: table` + `refresh: incremental` + `grain: key` instead. \
                 Note: 'materialized_view' has been removed — use `refresh: materialized_view` instead.",
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
    #[serde(default)]
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
    /// Project-level virtual-environment posture (D-47). Defaults to `stateless`
    /// (today's behaviour). Set to `intervals` or `environments` to enable the
    /// corresponding snapshot/reuse machinery.
    #[serde(default)]
    pub state: StateConfig,
    /// Project-level `maintenance:` baseline (today only `scan_bounds`) —
    /// the default every model's own `maintenance.scan_bounds` refines.
    /// See [`ProjectMaintenanceConfig`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<ProjectMaintenanceConfig>,
    /// Project-wide cadence policy for every declared-fact probe
    /// (`model_properties.md` §"Probe obligation"). Defaults to `per_run`.
    /// One policy governs every probe — per-declaration override is open
    /// (`smelt_yml.md` §Known Divergences).
    #[serde(default)]
    pub probes: ProbesConfig,
}

/// Opt-in state posture for virtual environments (D-47).
///
/// The three modes form a capability lattice: `environments ⊇ intervals ⊇ stateless`.
/// A model may narrow (declare a lower mode than the project) but not widen.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StateMode {
    /// Default: no `.smelt/` state store required; no snapshot reuse.
    #[default]
    Stateless,
    /// Persisted interval ledger for incremental models; no snapshot reuse.
    Intervals,
    /// Full virtual environments: fingerprint-keyed snapshot reuse + environment
    /// addressing.
    Environments,
}

impl StateMode {
    /// Returns the lowercase string representation of this mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            StateMode::Stateless => "stateless",
            StateMode::Intervals => "intervals",
            StateMode::Environments => "environments",
        }
    }

    /// Returns `true` if a model (or child project) with posture `self` may
    /// declare posture `target` — i.e. `target` is ≤ `self` in the lattice.
    ///
    /// `environments ⊇ intervals ⊇ stateless`, so narrowing moves down and
    /// widening (returning `false`) moves up.
    pub fn can_narrow_to(&self, target: &StateMode) -> bool {
        match (self, target) {
            // Environments can narrow to anything.
            (StateMode::Environments, _) => true,
            // Intervals can narrow to itself or stateless.
            (StateMode::Intervals, StateMode::Intervals | StateMode::Stateless) => true,
            (StateMode::Intervals, StateMode::Environments) => false,
            // Stateless can only stay stateless.
            (StateMode::Stateless, StateMode::Stateless) => true,
            (StateMode::Stateless, _) => false,
        }
    }
}

/// Whether smelt may create its own engine-resident bookkeeping tables in the
/// target backend (`state.warehouse_tables`, `docs/specs/state.md` §"Opting
/// out of warehouse bookkeeping"). `None` makes every engine-resident
/// correctness structure unavailable to availability resolution, downgrading
/// each cell that needed one to its recompute-family equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WarehouseTables {
    /// Default: engine-resident correctness structures are created as the
    /// derived plan needs them.
    #[default]
    Allowed,
    /// smelt authors no tables of its own in the target backend.
    None,
}

/// `state:` block in `smelt.yml` (D-47).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct StateConfig {
    #[serde(default)]
    pub mode: StateMode,
    #[serde(default)]
    pub warehouse_tables: WarehouseTables,
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

fn default_schema() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
    #[serde(rename = "type")]
    pub target_type: String,
    // DuckDB fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default = "default_schema")]
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
    /// Connection-time settings applied as `SET key = value` on open (DuckDB only).
    ///
    /// Each entry is applied in sorted key order immediately after the connection
    /// is opened and before the schema is created. Unknown keys are rejected with
    /// an error (fail-loud). Common keys: `memory_limit`, `threads`, `temp_directory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<BTreeMap<String, String>>,
    // BigQuery fields
    /// GCP project the BigQuery jobs are billed to and resolved against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// BigQuery dataset holding this target's tables — the analogue of a schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    /// Dataset location (e.g. `US`, `europe-west2`). Must match at query time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl Target {
    /// Get the backend type from the target_type field.
    ///
    /// Returns an error for unrecognised `type:` strings rather than silently
    /// defaulting — callers on the run path propagate this as a user-visible
    /// diagnostic ("unknown backend type `<x>`").
    pub fn backend_type(&self) -> anyhow::Result<BackendType> {
        match self.target_type.to_lowercase().as_str() {
            "duckdb" => Ok(BackendType::DuckDB),
            "spark" => Ok(BackendType::Spark),
            "bigquery" => Ok(BackendType::BigQuery),
            other => Err(anyhow::anyhow!("unknown backend type `{other}`")),
        }
    }

    /// Get the effective table format for this target.
    ///
    /// Returns `None` for DuckDB targets (format is not applicable).
    /// For Spark targets, defaults to `Delta` if not specified.
    /// Returns `None` for unrecognised backend types (the run path fails before
    /// format is needed).
    pub fn table_format(&self) -> Option<TableFormat> {
        match self.backend_type() {
            // Table format is a Spark concept; DuckDB and BigQuery each own their
            // storage and expose no choice.
            Ok(BackendType::DuckDB) | Ok(BackendType::BigQuery) | Err(_) => None,
            Ok(BackendType::Spark) => Some(self.format.unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    DuckDB,
    Spark,
    BigQuery,
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
    /// Refresh axis override (`full` | `incremental` | `materialized_view`).
    /// Frontmatter wins over this when both set it (see
    /// `Config::get_refresh`).
    #[serde(default)]
    pub refresh: Option<RefreshStrategy>,
    /// Declared grain (`partition` | `key` | `key_per_partition`) — an
    /// optional **check-only assertion**, validated against the derived
    /// shape facts rather than driving them. See [`Grain`],
    /// [`derive_grain`], and `docs/specs/models.md` §"Refresh axis".
    #[serde(default)]
    pub grain: Option<Grain>,
    /// The identity fact — top-level `unique_key:` (`docs/specs/models.md`
    /// §"Refresh axis", §"The Relation Contract"). A single string is sugar
    /// for a one-element list. Frontmatter wins over this smelt.yml override
    /// when both set it (see [`Config::get_unique_key_with_metadata`]).
    /// Distinct from `merge_key:` (a MERGE-dedup write key, never
    /// key-addressing).
    #[serde(default, deserialize_with = "crate::sources::opt_string_or_vec")]
    pub unique_key: Option<Vec<String>>,
    /// Top-level `safety_overrides:` (`docs/specs/models.md` §"The Relation
    /// Contract") — named escape hatches for the partition-grain safety
    /// checks, the smelt.yml-side replacement spelling for the retired
    /// `batched.safety_overrides` sub-block. Frontmatter's own top-level
    /// `safety_overrides:` wins over this wholesale — see
    /// [`Config::get_incremental_with_metadata`]. When smelt.yml is the only
    /// side declaring incremental config, this value is folded into the
    /// effective `batched:` block representation.
    #[serde(default)]
    pub safety_overrides: Option<PartitionGrainSafetyOverrides>,
    /// Retirement sentinel for the removed `models.<name>.batched:`
    /// sub-block (`docs/specs/models.md` §"Constraint violations").
    /// Presence of the `batched:` key on a smelt.yml model entry —
    /// regardless of value — is a hard parse error naming each declared
    /// sub-key's top-level replacement (`unique_key` → `merge_key:`,
    /// `safety_overrides` → `safety_overrides:`, `nondeterministic_columns`
    /// → `columns.<c>.contract: plausible`). Renamed from the former field
    /// so no consumer can read a stale value; never serialized.
    #[serde(
        default,
        rename = "batched",
        skip_serializing,
        deserialize_with = "reject_batched_subblock"
    )]
    pub batched_retired: (),
    /// The write/dedup key ([`PartitionGrainConfig::unique_key`]'s
    /// smelt.yml-side spelling) a column-scoped MERGE technique writes on —
    /// never the identity-conferring fact `unique_key:` is, and never a
    /// driver of grain. A single string is sugar for a one-element list.
    /// Frontmatter's own top-level `merge_key:` wins over this wholesale —
    /// see [`Config::get_incremental_with_metadata`]. Folded into the
    /// effective `batched:` block by [`fold_smelt_yml_incremental_keys`].
    #[serde(default, deserialize_with = "crate::sources::opt_string_or_vec")]
    pub merge_key: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Target to execute this model on (overrides CLI --target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Table format override for this model (Spark targets only).
    /// Precedence: SQL frontmatter `format:` > this field > target default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<TableFormat>,
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
///
/// Variant declaration order is increasing coarseness (`Hour` finest, `Year`
/// coarsest) and derives `PartialOrd`/`Ord` on that basis — `g_run >= g_part`
/// comparisons (`incremental_shapes.md` §"Run window vs partition granularity")
/// read this as a plain enum comparison rather than a bespoke arithmetic
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
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
pub struct PartitionGrainSafetyOverrides {
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
/// primitive used by the `refresh: incremental` + `grain: key` merge loop
/// (`docs/specs/incremental_models.md`), which is a separate sibling rule
/// with a different equivalence contract. `Backend::merge_into` remains on
/// the backend trait for that caller; it is not reachable from this enum.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalStrategy {
    DeleteInsert,
}

/// Time-dimension declaration for a model or source output.
///
/// Factored out of `PartitionGrainConfig` so that views, non-batched tables,
/// and external sources can declare a time dimension without opting into
/// incremental execution. `refresh: incremental` + `grain: partition` consumes this
/// block; any model
/// declaring `grain: partition` must also declare `timeseries:`.
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
    /// Declared-monotonicity escape hatch: the modeller's assertion that the
    /// projected `event_time` expression is monotone non-decreasing even
    /// though static analysis cannot decide it (an opaque UDF / opaque
    /// function body). Widens only the *undecidable* trace verdict — a
    /// positive disproof (a constant/NULL seed, or a row-nondeterministic
    /// value in the event-time position) is still refused; the declaration
    /// can never override those (`model_properties.md` §Constraints
    /// "Declared escape hatches may only widen").
    #[serde(default)]
    pub assert_monotonic: bool,
}

/// Model-scoped functional-dependency declaration: the modeller's assertion
/// that `determines` is a per-key constant for the given `key` columns — a
/// world-fact the model's own SQL cannot always decide statically. Licenses
/// once-write `COALESCE`/1:1-after-dedup enrichment (`model_properties.md`
/// §"Model-scoped declarations" row "Functional dependency (`key → column`)").
///
/// Widens only the *undecidable* per-key-constancy verdict: a `determines`
/// column that the fan-out/cardinality proof (`analysis::join_shape::fan_out`)
/// positively proves multi-valued per key is refused regardless of this
/// declaration (`model_properties.md` §Constraints "Declared escape hatches
/// may only widen").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionalDependency {
    /// Columns that functionally determine `determines` (the key side).
    pub key: Vec<String>,
    /// The column asserted to be a per-key constant under `key`.
    pub determines: String,
}

/// Model-scoped bounded-domain / space-budget declaration: the modeller's
/// assertion that `column`'s active domain is bounded by `max_cardinality`
/// distinct values — a world-fact the model's own SQL cannot decide
/// statically. Licenses an exact holistic aggregate (`MEDIAN`/`MODE`/exact
/// `COUNT(DISTINCT)`) for multiset maintenance via an explicit per-key
/// multiset (`model_properties.md` §"Model-scoped declarations" row
/// "Bounded-domain / space budget").
///
/// **Fail-loud with a cap, never the default.** `max_cardinality` is a
/// required field (no `#[serde(default)]`): an absent cap is a YAML parse
/// error, not a permissive default. `max_cardinality == 0` is additionally
/// rejected by `validate_bounded_domains` as a structural error, since a
/// zero-sized budget can never license anything.
///
/// Widens only the *holistic* aggregate-licence gate — a declaration applied
/// to a non-holistic (monoid/decomposable) combiner is refused/inert; it
/// cannot substitute for the fail-closed refusal of an unbounded domain, and
/// cannot narrow (`model_properties.md` §Constraints "Declared escape
/// hatches may only widen").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedDomain {
    /// The column whose active domain is asserted to be bounded.
    pub column: String,
    /// The explicit space budget: the maximum number of distinct values the
    /// column may take. Required — an absent cap is a parse-time error, not
    /// a silent default.
    pub max_cardinality: u64,
}

/// The `batched:` block — configuration layered on top of `refresh: incremental` +
/// `grain: partition`. Selection itself is that pair; this struct carries only
/// the optional knobs (`unique_key`, `safety_overrides`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionGrainConfig {
    /// Columns that uniquely identify a row (backend uses presence to choose strategy)
    #[serde(default)]
    pub unique_key: Vec<String>,
    /// Retirement sentinel for the removed `nondeterministic_columns` list
    /// form (`docs/specs/models.md` §"Constraint violations"). Presence of
    /// the `nondeterministic_columns` key under `smelt.yml`'s
    /// `models.<name>.batched:` block — regardless of value — is a hard
    /// parse error with a fix-it naming `columns.<c>.contract: plausible`
    /// in the model's `.sql` frontmatter, the sole surviving surface for the
    /// contract (there is no `smelt.yml` spelling). Renamed from the former
    /// field so no consumer can read a stale value; never serialized.
    #[serde(
        default,
        rename = "nondeterministic_columns",
        skip_serializing,
        deserialize_with = "reject_nondeterministic_columns"
    )]
    pub nondeterministic_columns_retired: (),
    /// Safety overrides for patterns that may diverge on partial data
    #[serde(default)]
    pub safety_overrides: PartitionGrainSafetyOverrides,
}

/// `deserialize_with` for [`PartitionGrainConfig::nondeterministic_columns_retired`]:
/// any presence of the `nondeterministic_columns` key — regardless of value
/// — is refused with a fix-it naming, per declared column, its sole
/// replacement.
fn reject_nondeterministic_columns<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let cols: Vec<String> = serde::Deserialize::deserialize(deserializer)?;
    let fixit = if cols.is_empty() {
        "columns.<c>.contract: plausible".to_string()
    } else {
        cols.iter()
            .map(|c| format!("columns.{c}.contract: plausible"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(D::Error::custom(format!(
        "`nondeterministic_columns` has been removed — declare `{fixit}` in the model's .sql \
         frontmatter instead (the contract has no `smelt.yml` spelling)"
    )))
}

/// `deserialize_with` for [`ModelConfig::batched_retired`]: any presence of
/// the `batched:` key on a smelt.yml model entry — regardless of value — is
/// refused with a fix-it naming each declared sub-key's top-level
/// replacement and the caller's own value (`docs/specs/models.md`
/// §"Constraint violations").
fn reject_batched_subblock<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: serde_yaml::Value = serde::Deserialize::deserialize(deserializer)?;
    let header = "the `batched:` sub-block has been removed — declare each key at the \
                  model's top level instead:";
    let generic = format!(
        "{header}\n  - `batched.unique_key` -> top-level `merge_key:`\n  - \
         `batched.safety_overrides` -> top-level `safety_overrides:`\n  - \
         `batched.nondeterministic_columns: [c]` -> `columns.c.contract: plausible`"
    );
    let Some(map) = raw.as_mapping() else {
        return Err(D::Error::custom(generic));
    };

    let mut lines = vec![header.to_string()];
    if let Some(v) = map.get(serde_yaml::Value::String("unique_key".to_string())) {
        let cols: Vec<String> = serde_yaml::from_value(v.clone()).unwrap_or_default();
        lines.push(format!(
            "  - `batched.unique_key: {:?}` -> top-level `merge_key: {:?}`",
            cols, cols
        ));
    }
    if let Some(v) = map.get(serde_yaml::Value::String("safety_overrides".to_string())) {
        lines.push(format!(
            "  - `batched.safety_overrides: {:?}` -> top-level `safety_overrides: {:?}`",
            v, v
        ));
    }
    if let Some(v) = map.get(serde_yaml::Value::String(
        "nondeterministic_columns".to_string(),
    )) {
        let cols: Vec<String> = serde_yaml::from_value(v.clone()).unwrap_or_default();
        if cols.is_empty() {
            lines.push(
                "  - `batched.nondeterministic_columns` -> `columns.<c>.contract: plausible`"
                    .to_string(),
            );
        } else {
            for col in &cols {
                lines.push(format!(
                    "  - `batched.nondeterministic_columns: [{col}]` -> `columns.{col}.contract: plausible`"
                ));
            }
        }
    }

    if lines.len() == 1 {
        return Err(D::Error::custom(generic));
    }
    Err(D::Error::custom(lines.join("\n")))
}

/// The `maintenance:` block (`incremental_models.md` §Surface "Frontmatter"):
/// per-cell technique preferences/pins and the scan-locality guardrail.
/// Almost every model sets none of it — the plan derives cells, clamps, and
/// locality verdicts on its own; this block only *constrains* the derived
/// plan (a soft bias, a hard pin, or a refusal guardrail), never chooses a
/// strategy the derivation didn't already admit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceConfig {
    /// Per-model soft default technique preference (`auto` = cost model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<MaintenanceDefaults>,
    /// Per-cell overrides, keyed by the columns + trigger they address.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<MaintenanceCellConfig>,
    /// The partition-locality guardrail (the K8 default: `require:
    /// partition_local`, `on_violation: error`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_bounds: Option<ScanBoundsConfig>,
}

/// The `contract:` block (`incremental_models.md` §"Contract relaxations
/// (`contract:`)"): a declared relaxation of the equivalence invariant — the
/// default point in the contract lattice. Absent means the default point
/// (strict equivalence).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractConfig {
    /// Partitions older than `end − frozen_horizon` are never revisited by
    /// maintenance; admitted only on a partition-grain model (checked by
    /// `smelt_logical::contract::frozen_horizon::validate_frozen_horizon`,
    /// which needs the derived grain this pure serde shape does not have).
    /// Format validity (a parseable interval) is checked at frontmatter-parse
    /// time in `smelt_core::metadata`, raising
    /// `MetadataError::ContractFrozenHorizonInvalid` rather than the generic
    /// YAML parse error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_horizon: Option<DataLatency>,
    /// Model-level default deferral window `D`: the maintained state may lag
    /// its inputs by up to `D`, licensing run skipping and work subsumption
    /// (`incremental_models.md` §"The contract lattice"). Admitted only when
    /// the model carries a `timeseries:` clock (checked by
    /// `smelt_logical::contract::deferral::validate_deferral`, which needs
    /// the parsed `ModelMetadata` this pure serde shape does not have).
    /// Format validity is checked at frontmatter-parse time, raising
    /// `MetadataError::ContractDeferralInvalid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferral: Option<DataLatency>,
    /// `retain_departed: true` or `retain_departed: {tombstone: <col>}` —
    /// keeps keys the source no longer carries instead of deleting them at
    /// reconcile; admitted only on a keyed shape consuming a mutable
    /// snapshot (checked by
    /// `smelt_logical::contract::retain_departed::validate`, which needs
    /// the derived grain, resolved source facts, and inferred output
    /// schema this pure serde shape does not have). A value that is
    /// neither a bare bool nor `{tombstone: <col>}`, or a `tombstone`
    /// naming a column absent from the model's output, is a dedicated
    /// `MetadataError::ContractRetainDepartedInvalid` rather than the
    /// generic YAML parse error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retain_departed: Option<RetainDeparted>,
    /// Per-cell refinement, addressed like `maintenance.cells[]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<ContractCellConfig>,
}

/// The two admitted `contract.retain_departed` declaration forms
/// (`incremental_models.md` §"Contract relaxations (`contract:`)"): bare
/// retention, or retention with departure marked in a named tombstone
/// column.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RetainDeparted {
    Bool(bool),
    Tombstone { tombstone: String },
}

/// One `contract.cells[]` entry: a per-`(columns × trigger)` `deferral`
/// override, addressed the same way as `maintenance.cells[]`
/// (`incremental_models.md` §"Contract relaxations (`contract:`)").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractCellConfig {
    /// Names any member of the derived column group this cell addresses.
    pub columns: Vec<String>,
    /// The trigger this cell handles: a `<source-address>` or the literal
    /// `backfill`.
    pub on: String,
    /// This cell's deferral window `D`, overriding the model-level default.
    /// Admitted only when `on:` addresses a clocked, interval-representable
    /// source — `on: backfill`, an unclocked source, and a
    /// `mutable_snapshot` source each raise
    /// `MetadataError::ContractDeferralInvalid` /
    /// `DiagnosticCode::ContractDeferralInvalid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferral: Option<DataLatency>,
}

/// `maintenance.defaults` — the per-model soft technique bias.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<TechniquePreference>,
}

/// A soft/hard technique bias value — shared by `defaults.prefer` and
/// `cells[].prefer`. `technique:` pins (bypassing the cost model) reuse
/// [`CellTechnique`] instead, since it additionally admits
/// `rederive_columns`.
///
/// `Suppress`/`Unconditional` are a second, orthogonal bias dimension folded
/// onto the same `prefer:` key rather than a new declared field: which
/// matched-arm *variant* a suppressible cell's already-chosen family
/// (`ColumnScopedMerge`/keyed fold) writes, independent of the `Fold`/
/// `Recompute` family choice above
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// G1; `docs/research/20260715-conditional-maintenance-without-cdf.md` item
/// 8: conditional variants change *which technique serves a cell*,
/// "steerable via `maintenance:` prefer/pin" — no new declared model
/// surface). Meaningful only when the resolved family is suppressible; a
/// value here never changes family choice itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TechniquePreference {
    Fold,
    Recompute,
    /// Soft bias toward the change-suppressed matched-arm variant, e.g. to
    /// override the first-build/definition-change-backfill posture's
    /// default of not preferring it.
    Suppress,
    /// Soft bias toward the plain unconditional matched arm, overriding the
    /// steady-state trigger's default preference for suppression.
    Unconditional,
    /// The cost model decides (the default when `defaults.prefer` is absent).
    Auto,
}

/// One `maintenance.cells[]` entry: a per-`(columns × trigger)` override.
/// `columns` naming members of more than one derived column group is an
/// error — it would silently re-partition the plan
/// (`incremental_models.md` §Surface "Frontmatter").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceCellConfig {
    /// Names any member of the derived column group this cell addresses.
    pub columns: Vec<String>,
    /// The trigger this cell handles: a `<source-address>` or the literal
    /// `backfill`.
    pub on: String,
    /// Soft bias — the cost model may still choose a different admissible
    /// technique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<TechniquePreference>,
    /// Hard pin — bypasses the cost model, but never bypasses admission (a
    /// pin naming an unadmitted technique is an error, not an override).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technique: Option<CellTechnique>,
    /// Hard per-cell **addressing** pin (`incremental_models.md` §"Per-cell
    /// write addressing" → "User pins"): an **open name** resolved against
    /// the write-pattern registry (`smelt_logical::maintenance::
    /// lookup_write_pattern`), not a sealed keyword set — deliberately
    /// `Option<String>`, not an enum, so a new backend-contributed pattern
    /// name is admitted the moment it registers, with no `smelt-core`
    /// release required. An unrecognised name, or one the target backend
    /// cannot execute, is `MaintenanceWritePatternUnavailable`; a
    /// registry-recognised, backend-capable name whose addressing cannot
    /// uphold this cell's equivalence invariant is
    /// `MaintenanceWriteAddressingRefused` — both validated downstream
    /// (`smelt-db`'s maintenance-plan diagnostics), never here (this struct
    /// only parses the open string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<String>,
}

/// `maintenance.cells[].technique` — the hard-pin value set (a superset of
/// [`TechniquePreference`]: `rederive_columns` is only meaningful as an
/// explicit pin, never a soft bias).
///
/// `Suppress`/`Unconditional` mirror [`TechniquePreference`]'s own pair: the
/// same orthogonal write-suppression dimension, but as a hard pin — never a
/// family pin (it does not select `Fold`/`Recompute`/`RederiveColumns`).
/// Forcing `Suppress` on a cell whose write-suppression proof (P2/P3)
/// itself refused is a hard, fail-loud refusal, exactly like pinning a
/// family the derived plan never admitted — a pin bypasses the cost model,
/// never the admission proof. `Unconditional` never refuses: the plain
/// matched arm is always a safe fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellTechnique {
    Fold,
    Recompute,
    RederiveColumns,
    /// Force the change-suppressed matched-arm variant on, bypassing the
    /// first-build/definition-change-backfill posture's default.
    Suppress,
    /// Force the plain unconditional matched arm, bypassing the
    /// steady-state trigger's default preference for suppression.
    Unconditional,
}

/// The partition-locality guardrail (`incremental_models.md` §Semantics "The
/// K8 guardrail"). A project-level block in `smelt.yml` sets the baseline;
/// a per-model block refines it (narrower wins, exactly like the technique
/// ladder). Check-only: never modifies a derived clamp, only refuses (or
/// warns) when the derived plan exceeds the stated expectation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanBoundsConfig {
    /// Default (when absent): `partition_local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require: Option<ScanBoundsRequire>,
    /// Default (when absent): `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_violation: Option<ScanBoundsViolation>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub per_source: HashMap<String, PerSourceScanBounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanBoundsRequire {
    PartitionLocal,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanBoundsViolation {
    Error,
    Warn,
}

/// A named per-source acceptance/ceiling under `maintenance.scan_bounds`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PerSourceScanBounds {
    /// Ceiling on the derived scan span for this source. Parsed but not yet
    /// checked against the derived clamp (`incremental_models.md` §Known
    /// Divergences) — reserved for a future phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lookback: Option<String>,
    /// Named acceptance of a full read of this source.
    #[serde(default)]
    pub allow_full_scan: bool,
}

impl ScanBoundsConfig {
    /// The effective `require` policy, defaulting to `partition_local`.
    pub fn require(&self) -> ScanBoundsRequire {
        self.require.unwrap_or(ScanBoundsRequire::PartitionLocal)
    }

    /// Whether `source_address` is named with `allow_full_scan: true`.
    pub fn allow_full_scan(&self, source_address: &str) -> bool {
        self.per_source
            .get(source_address)
            .is_some_and(|p| p.allow_full_scan)
    }
}

/// Project-level `maintenance:` block in `smelt.yml` — today only the
/// `scan_bounds` baseline (`incremental_models.md` §Surface "Frontmatter":
/// "A project-level default in `smelt.yml` sets the baseline; per-model
/// blocks refine it").
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMaintenanceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_bounds: Option<ScanBoundsConfig>,
}

/// `probes:` (`smelt_yml.md` §"Top-level keys") — the project-wide cadence
/// policy governing every declared-fact probe (`model_properties.md`
/// §"Probe obligation"). Custom `Deserialize` (rather than a plain derive)
/// because `periodic` cross-validates against `cadence`: a `periodic`
/// cadence without a positive `every_n_runs` is a configuration error, not
/// a silent default (root `CLAUDE.md` §"Fail-loud discipline").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProbesConfig {
    pub cadence: ProbeCadence,
}

impl Default for ProbesConfig {
    fn default() -> Self {
        ProbesConfig {
            cadence: ProbeCadence::PerRun,
        }
    }
}

/// The resolved probe-dispatch cadence
/// (`smelt-logical::maintenance::probe_cadence::should_dispatch` consumes
/// this directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCadence {
    /// Dispatch every `built` probe on every consuming run (the default).
    PerRun,
    /// Dispatch once every `every_n_runs` runs (ordinal 0 always dispatches).
    Periodic { every_n_runs: u32 },
    /// Never dispatch — every declaration is trusted and recorded
    /// unverified on the run manifest.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProbeCadenceKind {
    #[default]
    PerRun,
    Periodic,
    Off,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPeriodicProbeConfig {
    every_n_runs: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProbesConfig {
    #[serde(default)]
    cadence: ProbeCadenceKind,
    #[serde(default)]
    periodic: Option<RawPeriodicProbeConfig>,
}

impl<'de> Deserialize<'de> for ProbesConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawProbesConfig::deserialize(deserializer)?;
        let cadence = match raw.cadence {
            ProbeCadenceKind::PerRun => ProbeCadence::PerRun,
            ProbeCadenceKind::Off => ProbeCadence::Off,
            ProbeCadenceKind::Periodic => {
                let every_n_runs = raw.periodic.map(|p| p.every_n_runs).ok_or_else(|| {
                    serde::de::Error::custom(
                        "probes: cadence: periodic requires a `periodic.every_n_runs` block",
                    )
                })?;
                if every_n_runs == 0 {
                    return Err(serde::de::Error::custom(
                        "probes.periodic.every_n_runs must be greater than 0",
                    ));
                }
                ProbeCadence::Periodic { every_n_runs }
            }
        };
        Ok(ProbesConfig { cadence })
    }
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

/// Resolve `${VAR}` environment-variable references in raw `smelt.yml` text,
/// before it is parsed into a typed `Config` (`smelt_yml.md` §Semantics item
/// 8 — interpolation runs exactly once, in the config-load pass, before any
/// other parse-time or semantic validation observes the value).
///
/// Walks the generic YAML value tree (not the raw text) so each reference
/// can be reported against its YAML key path (e.g. `targets.prod.connect_url`).
/// `$$` is unescaped to a literal `$` in the same pass. Every unresolved
/// `${VAR}` in the file is collected and reported together — a config with
/// two missing variables names both, never just the first. `env_lookup` is
/// injectable so tests don't have to mutate process env.
///
/// If `text` does not parse as YAML at all, interpolation is skipped and the
/// text is returned unchanged — the downstream `Config` parse produces the
/// real syntax error instead of a confusing interpolation-stage one.
pub fn interpolate_env_vars(
    text: &str,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<String> {
    let value: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(_) => return Ok(text.to_string()),
    };
    let mut missing: Vec<(String, String)> = Vec::new();
    let interpolated = interpolate_value(value, String::new(), env_lookup, &mut missing);
    if !missing.is_empty() {
        let mut detail = String::new();
        for (var, key_path) in &missing {
            detail.push_str(&format!(
                "  ${{{var}}} at `{key_path}` — environment variable `{var}` is not set\n"
            ));
        }
        anyhow::bail!(
            "smelt.yml: unresolved environment variable reference(s):\n{}",
            detail.trim_end()
        );
    }
    serde_yaml::to_string(&interpolated)
        .map_err(|e| anyhow::anyhow!("failed to re-serialize smelt.yml after interpolation: {e}"))
}

fn interpolate_value(
    value: serde_yaml::Value,
    key_path: String,
    env_lookup: &dyn Fn(&str) -> Option<String>,
    missing: &mut Vec<(String, String)>,
) -> serde_yaml::Value {
    match value {
        serde_yaml::Value::String(s) => {
            serde_yaml::Value::String(interpolate_string(&s, &key_path, env_lookup, missing))
        }
        serde_yaml::Value::Mapping(map) => {
            let mut new_map = serde_yaml::Mapping::new();
            for (k, v) in map {
                let child_path = match (k.as_str(), key_path.is_empty()) {
                    (Some(name), true) => name.to_string(),
                    (Some(name), false) => format!("{key_path}.{name}"),
                    (None, _) => key_path.clone(),
                };
                let new_v = interpolate_value(v, child_path, env_lookup, missing);
                new_map.insert(k, new_v);
            }
            serde_yaml::Value::Mapping(new_map)
        }
        serde_yaml::Value::Sequence(seq) => serde_yaml::Value::Sequence(
            seq.into_iter()
                .enumerate()
                .map(|(i, v)| interpolate_value(v, format!("{key_path}[{i}]"), env_lookup, missing))
                .collect(),
        ),
        other => other,
    }
}

/// Interpolate `${VAR}` / unescape `$$` within a single string leaf. Any
/// unresolved reference is pushed to `missing` (keyed by variable name and
/// the string's YAML key path) rather than resolved to an empty string.
fn interpolate_string(
    s: &str,
    key_path: &str,
    env_lookup: &dyn Fn(&str) -> Option<String>,
    missing: &mut Vec<(String, String)>,
) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
            out.push('$');
            i += 2;
            continue;
        }
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            if let Some(rel_end) = chars[i + 2..].iter().position(|&c| c == '}') {
                let name: String = chars[i + 2..i + 2 + rel_end].iter().collect();
                match env_lookup(&name) {
                    Some(val) => out.push_str(&val),
                    None => missing.push((name, key_path.to_string())),
                }
                i = i + 2 + rel_end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Fold a smelt.yml model entry's top-level `merge_key:`
/// ([`ModelConfig::merge_key`]) and `safety_overrides:`
/// ([`ModelConfig::safety_overrides`]) into the effective `batched:` block
/// representation ([`PartitionGrainConfig`]), the smelt.yml-side mirror of
/// `metadata::fold_top_level_merge_key` / `metadata::fold_top_level_safety_overrides`
/// for SQL frontmatter. Called from [`Config::get_incremental`] /
/// [`Config::get_incremental_with_metadata`] so every `batched:`-shaped
/// consumer sees the top-level spellings identically to the retired
/// sub-block form.
fn fold_smelt_yml_incremental_keys(model_config: &ModelConfig) -> PartitionGrainConfig {
    let mut batched = PartitionGrainConfig::default();
    if let Some(merge_key) = &model_config.merge_key {
        batched.unique_key = merge_key.clone();
    }
    if let Some(top_level) = &model_config.safety_overrides {
        batched.safety_overrides = top_level.clone();
    }
    batched
}

impl Config {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join("smelt.yml");
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| ConfigError::LoadError {
                path: config_path.clone(),
                source: e.into(),
            })?;

        let interpolated = interpolate_env_vars(&content, &|name| std::env::var(name).ok())
            .map_err(|e| ConfigError::LoadError {
                path: config_path.clone(),
                source: e,
            })?;

        let (config, warnings) =
            Self::parse_with_warnings(&interpolated).map_err(|e| ConfigError::LoadError {
                path: config_path,
                source: e.into(),
            })?;
        for w in &warnings {
            warn!("{}", w);
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
                // `vars` is consumed by `smelt-db::config_vars::parse_vars_from_yaml`
                // (compile-time `smelt.config.var` variables), also not a `Config` field.
                const KNOWN_KEYS: &[&str] = &[
                    "name",
                    "version",
                    "paths",
                    "targets",
                    "target",
                    "default_materialization",
                    "models",
                    "python",
                    "model_paths",
                    "seed_paths",
                    "unstable_schema",
                    "vars",
                    "state",
                    "probes",
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

    /// Get the effective refresh strategy for a model (smelt.yml only).
    ///
    /// **Precedence**: smelt.yml only (for now). Use
    /// [`Config::get_refresh_with_metadata`] to also consider SQL frontmatter.
    pub fn get_refresh(&self, model_name: &str) -> RefreshStrategy {
        self.models
            .get(model_name)
            .and_then(|m| m.refresh.clone())
            .unwrap_or(RefreshStrategy::Full)
    }

    /// Get the effective refresh strategy for a model.
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config > `Full`.
    pub fn get_refresh_with_metadata(
        &self,
        model_name: &str,
        sql_metadata: Option<&ModelMetadata>,
    ) -> RefreshStrategy {
        if let Some(metadata) = sql_metadata {
            if let Some(refresh) = &metadata.refresh {
                return refresh.clone();
            }
        }
        self.get_refresh(model_name)
    }

    /// Get the declared `grain:` for a model, from smelt.yml only.
    ///
    /// **Precedence**: smelt.yml only (for now). Use
    /// [`Config::get_grain_with_metadata`] to also consider SQL frontmatter.
    pub fn get_grain(&self, model_name: &str) -> Option<Grain> {
        self.models.get(model_name).and_then(|m| m.grain)
    }

    /// Get the declared `grain:` for a model.
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config.
    pub fn get_grain_with_metadata(
        &self,
        model_name: &str,
        sql_metadata: Option<&ModelMetadata>,
    ) -> Option<Grain> {
        if let Some(metadata) = sql_metadata {
            if let Some(grain) = metadata.grain {
                return Some(grain);
            }
        }
        self.get_grain(model_name)
    }

    /// Get the declared top-level `unique_key:` for a model, from smelt.yml only.
    ///
    /// **Precedence**: smelt.yml only (for now). Use
    /// [`Config::get_unique_key_with_metadata`] to also consider SQL frontmatter.
    pub fn get_unique_key(&self, model_name: &str) -> Option<&[String]> {
        self.models
            .get(model_name)
            .and_then(|m| m.unique_key.as_deref())
    }

    /// Get the declared top-level `unique_key:` for a model.
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config.
    pub fn get_unique_key_with_metadata<'a>(
        &'a self,
        model_name: &str,
        sql_metadata: Option<&'a ModelMetadata>,
    ) -> Option<&'a [String]> {
        if let Some(metadata) = sql_metadata {
            if let Some(unique_key) = metadata.unique_key.as_deref() {
                return Some(unique_key);
            }
        }
        self.get_unique_key(model_name)
    }

    /// Get the `batched:` block for a model, when the model is selected into
    /// partition-grain incremental refresh (`refresh: incremental` +
    /// `grain: partition` — the former `refresh: batched`), from smelt.yml
    /// only.
    ///
    /// The opt-in is `refresh: incremental` + `grain: partition`, not the
    /// presence of the `batched:` block — a selected model with no block
    /// returns `Some(default)`.
    ///
    /// **Precedence**: smelt.yml only (for now).
    pub fn get_incremental(&self, model_name: &str) -> Option<PartitionGrainConfig> {
        if !matches!(self.get_refresh(model_name), RefreshStrategy::Incremental)
            || self.get_grain(model_name) != Some(Grain::Partition)
        {
            return None;
        }
        Some(
            self.models
                .get(model_name)
                .map(fold_smelt_yml_incremental_keys)
                .unwrap_or_default(),
        )
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

    /// Get table format for a model using three-tier precedence.
    ///
    /// **Precedence**: SQL frontmatter `format:` > `smelt.yml` `models.<name>.format` > target default.
    /// DuckDB targets always return `None` — format is not applicable.
    pub fn get_format(
        &self,
        model_name: &str,
        sql_metadata: Option<&ModelMetadata>,
        target: &Target,
    ) -> Option<TableFormat> {
        if !matches!(target.backend_type(), Ok(BackendType::Spark)) {
            return None;
        }
        if let Some(meta) = sql_metadata {
            if let Some(fmt) = meta.format {
                return Some(fmt);
            }
        }
        if let Some(model_config) = self.models.get(model_name) {
            if let Some(fmt) = model_config.format {
                return Some(fmt);
            }
        }
        target.table_format()
    }

    /// Get the `batched:` block for a model, when the model is selected into
    /// partition-grain incremental refresh (`refresh: incremental` +
    /// `grain: partition` — the former `refresh: batched`), with SQL
    /// metadata precedence.
    ///
    /// The opt-in is `refresh: incremental` + `grain: partition`
    /// (frontmatter wins over smelt.yml, see
    /// [`Config::get_refresh_with_metadata`] /
    /// [`Config::get_grain_with_metadata`]), not the presence of the
    /// `batched:` block — a selected model with no block returns
    /// `Some(default)`.
    ///
    /// **Precedence**: SQL file metadata > smelt.yml model config
    pub fn get_incremental_with_metadata(
        &self,
        model_name: &str,
        sql_metadata: Option<&ModelMetadata>,
    ) -> Option<PartitionGrainConfig> {
        if !matches!(
            self.get_refresh_with_metadata(model_name, sql_metadata),
            RefreshStrategy::Incremental
        ) || self.get_grain_with_metadata(model_name, sql_metadata) != Some(Grain::Partition)
        {
            return None;
        }
        if let Some(metadata) = sql_metadata {
            if let Some(batched) = &metadata.batched {
                return Some(batched.clone());
            }
        }
        Some(
            self.models
                .get(model_name)
                .map(fold_smelt_yml_incremental_keys)
                .unwrap_or_default(),
        )
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

        // Collect all model names and their effective materialization + config.
        // The smelt.yml-side dual-declaration check between top-level
        // `safety_overrides:` and `batched.safety_overrides` is gone: the
        // literal `batched:` sub-block is refused at parse time
        // (`reject_batched_subblock`), so a `ModelConfig` can never carry
        // both spellings at once — the conflict this used to catch is now
        // unreachable.
        let mut all_models: HashMap<
            &str,
            (Materialization, Option<PartitionGrainConfig>, Option<&str>),
        > = HashMap::new();

        // From smelt.yml
        for (name, model_config) in &self.models {
            let mat = model_config
                .materialization
                .clone()
                .unwrap_or_else(|| self.default_materialization.clone());
            let incremental =
                if model_config.merge_key.is_some() || model_config.safety_overrides.is_some() {
                    Some(fold_smelt_yml_incremental_keys(model_config))
                } else {
                    None
                };
            all_models.insert(
                name.as_str(),
                (mat, incremental, model_config.target.as_deref()),
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
            if let Some(inc) = &metadata.batched {
                entry.1 = Some(inc.clone());
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
                    if incremental.is_some() {
                        warn!(
                            "model '{}' is a view but has batched config — batched refresh only applies to tables",
                            name
                        );
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

    /// A `bigquery` target names project/dataset/location in place of DuckDB's
    /// `database` or Spark's `connect_url` (`multi_backend.md` §Surface).
    #[test]
    fn bigquery_target_parses_project_dataset_location() {
        let yaml = r#"
name: test_project
targets:
  bq:
    type: bigquery
    project: my-gcp-project
    dataset: analytics
    location: US
    schema: analytics
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("bigquery target must parse");
        let target = &config.targets["bq"];
        assert!(matches!(
            target.backend_type().expect("bigquery is a known type"),
            BackendType::BigQuery
        ));
        assert_eq!(target.project.as_deref(), Some("my-gcp-project"));
        assert_eq!(target.dataset.as_deref(), Some("analytics"));
        assert_eq!(target.location.as_deref(), Some("US"));
    }

    /// Table format is a Spark concept; a BigQuery target has none, exactly as
    /// a DuckDB target has none.
    #[test]
    fn bigquery_target_has_no_table_format() {
        let target = Target {
            target_type: "bigquery".to_string(),
            database: None,
            schema: "analytics".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
            project: Some("p".to_string()),
            dataset: Some("d".to_string()),
            location: None,
        };
        assert_eq!(target.table_format(), None);
    }

    /// `${VAR}` in a target field resolves against a set (injected) variable.
    #[test]
    fn env_interpolation_resolves_var() {
        let yaml = r#"
name: test_project
targets:
  prod:
    type: spark
    connect_url: sc://${SPARK_HOST}:15002
"#;
        let lookup = |name: &str| -> Option<String> {
            if name == "SPARK_HOST" {
                Some("spark-cluster.internal".to_string())
            } else {
                None
            }
        };
        let resolved =
            interpolate_env_vars(yaml, &lookup).expect("SPARK_HOST is set, must resolve");
        let config: Config = serde_yaml::from_str(&resolved).expect("resolved YAML must parse");
        assert_eq!(
            config.targets["prod"].connect_url.as_deref(),
            Some("sc://spark-cluster.internal:15002")
        );
    }

    /// A missing variable is a hard error naming both the variable and the
    /// YAML key path it appears under — never a silent empty string.
    #[test]
    fn env_interpolation_missing_var_is_error() {
        let yaml = r#"
name: test_project
targets:
  prod:
    type: spark
    connect_url: sc://${SPARK_HOST}:15002
"#;
        let lookup = |_: &str| -> Option<String> { None };
        let err =
            interpolate_env_vars(yaml, &lookup).expect_err("unset SPARK_HOST must be a hard error");
        let message = err.to_string();
        assert!(
            message.contains("SPARK_HOST"),
            "error must name the missing variable: {message}"
        );
        assert!(
            message.contains("targets.prod.connect_url"),
            "error must name the YAML key path: {message}"
        );
    }

    /// `$$` unescapes to a literal `$` with no lookup attempted, even when
    /// no environment variables are set at all.
    #[test]
    fn env_interpolation_double_dollar_escapes() {
        let yaml = r#"
name: test_project
targets:
  dev:
    type: duckdb
    database: "cost$$report.duckdb"
    schema: main
"#;
        let lookup = |_: &str| -> Option<String> { None };
        let resolved =
            interpolate_env_vars(yaml, &lookup).expect("no ${VAR} reference, must not error");
        let config: Config = serde_yaml::from_str(&resolved).expect("resolved YAML must parse");
        assert_eq!(
            config.targets["dev"].database.as_deref(),
            Some("cost$report.duckdb")
        );
    }

    /// W4·P2: `connect_url`'s token/TLS parameters interpolate like any other
    /// target field — no bespoke Spark-only interpolation path exists.
    #[test]
    fn spark_connect_url_interpolates_env_var() {
        let yaml = r#"
name: test_project
targets:
  prod:
    type: spark
    connect_url: sc://h:443/;token=${SMELT_TEST_TOKEN};use_ssl=true
"#;
        let lookup = |name: &str| -> Option<String> {
            if name == "SMELT_TEST_TOKEN" {
                Some("secret-token".to_string())
            } else {
                None
            }
        };
        let resolved =
            interpolate_env_vars(yaml, &lookup).expect("SMELT_TEST_TOKEN is set, must resolve");
        let config: Config = serde_yaml::from_str(&resolved).expect("resolved YAML must parse");
        assert_eq!(
            config.targets["prod"].connect_url.as_deref(),
            Some("sc://h:443/;token=secret-token;use_ssl=true")
        );
    }

    /// W4·P2: an unset token variable in `connect_url` fails loud, naming the
    /// variable and the key path — never a silently empty token.
    #[test]
    fn spark_connect_url_missing_token_is_error() {
        let yaml = r#"
name: test_project
targets:
  prod:
    type: spark
    connect_url: sc://h:443/;token=${SMELT_TEST_TOKEN};use_ssl=true
"#;
        let lookup = |_: &str| -> Option<String> { None };
        let err = interpolate_env_vars(yaml, &lookup)
            .expect_err("unset SMELT_TEST_TOKEN must be a hard error");
        let message = err.to_string();
        assert!(
            message.contains("SMELT_TEST_TOKEN"),
            "error must name the missing variable: {message}"
        );
        assert!(
            message.contains("targets.prod.connect_url"),
            "error must name the YAML key path: {message}"
        );
    }

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

    /// Phase 6: `materialization: test` is no longer a valid surface.
    /// Tests are declared with `smelt.test` in the SQL body, not via the
    /// `materialization:` frontmatter key.
    #[test]
    fn materialization_test_rejected() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  some_test:
    materialization: test
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "`materialization: test` must be rejected as an unknown value"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid materialization") || err.contains("unknown variant"),
            "error must mention the invalid value; got: {err}"
        );
    }

    #[test]
    fn test_materialization_cumulative_aggregate_is_rejected() {
        // `materialization: cumulative_aggregate` is no longer valid —
        // use `materialization: table` + `refresh: incremental` + `grain: key` instead.
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
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "`materialization: cumulative_aggregate` must be rejected as an unknown value"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cumulative_aggregate") || err.contains("Invalid materialization"),
            "error must mention the invalid value; got: {err}"
        );
    }

    /// `refresh: incremental` + `grain: key` models with an internally-folded
    /// `batched` block emit `PartitionGrainRequiresRefreshIncremental` — the dedicated
    /// `KeyedForbidsPartitionGrain` check was removed as unreachable once the
    /// literal `batched:` sub-block became a parse-time refusal (`is_keyed()`
    /// implies `!is_partition_grain()`, which is exactly what
    /// `PartitionGrainRequiresRefreshIncremental` already checks). Since
    /// `materialization: cumulative_aggregate` is no longer accepted, this
    /// test uses the new surface (`materialization: table` +
    /// `refresh: incremental` + `grain: key`).
    #[test]
    fn test_validate_refresh_keyed_forbids_incremental_via_metadata() {
        use crate::config::{
            Grain, PartitionGrainConfig, PartitionGrainSafetyOverrides, RefreshStrategy,
        };
        use crate::metadata::{validate_timeseries, MetadataError, ModelMetadata};

        let metadata = ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(Grain::Key),
            batched: Some(PartitionGrainConfig {
                unique_key: vec![],
                nondeterministic_columns_retired: (),
                safety_overrides: PartitionGrainSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT * FROM foo")
            .expect_err("refresh: incremental + grain: key + batched: must error");
        assert!(
            matches!(err, MetadataError::KeyedForbidsSafetyOverrides),
            "Expected KeyedForbidsSafetyOverrides, got: {}",
            err
        );
    }

    /// `refresh: incremental` deserializes to `RefreshStrategy::Incremental`.
    #[test]
    fn test_refresh_strategy_incremental_deserializes() {
        let strategy: RefreshStrategy = serde_yaml::from_str("incremental").unwrap();
        assert_eq!(strategy, RefreshStrategy::Incremental);
    }

    /// `refresh: cumulative` is a hard error pointing at the renamed value.
    #[test]
    fn test_refresh_strategy_cumulative_is_hard_error() {
        let result: Result<RefreshStrategy, _> = serde_yaml::from_str("cumulative");
        let err = result
            .expect_err("`refresh: cumulative` must be rejected")
            .to_string();
        assert!(
            err.contains("refresh: incremental") && err.contains("grain:"),
            "error must name the refresh: incremental + grain: replacement; got: {err}"
        );
    }

    /// `refresh: batched`/`keyed` are hard errors pointing at the
    /// `refresh: incremental` + `grain:` replacement; `refresh: versioned` is a
    /// hard error pointing at the plain-SQL SCD2 posture
    /// (`docs/specs/incremental_models.md` §Limitations).
    #[test]
    fn test_refresh_strategy_removed_names_are_hard_errors() {
        for value in ["batched", "keyed"] {
            let result: Result<RefreshStrategy, _> = serde_yaml::from_str(value);
            let err = result
                .expect_err("removed refresh name must be rejected")
                .to_string();
            assert!(
                err.contains("refresh: incremental") && err.contains("grain:"),
                "error for '{value}' must name the refresh: incremental + grain: replacement; got: {err}"
            );
        }

        let result: Result<RefreshStrategy, _> = serde_yaml::from_str("versioned");
        let err = result
            .expect_err("removed refresh name must be rejected")
            .to_string();
        assert!(
            err.contains("refresh: full") && err.contains("Limitations"),
            "error for 'versioned' must steer to the plain-SQL SCD2 posture; got: {err}"
        );
    }

    /// `refresh: latest_value` and `refresh: accumulating_snapshot` remain
    /// unknown-value errors — this rename does not introduce them as aliases.
    #[test]
    fn test_refresh_strategy_latest_value_and_accumulating_snapshot_remain_unknown() {
        for value in ["latest_value", "accumulating_snapshot"] {
            let result: Result<RefreshStrategy, _> = serde_yaml::from_str(value);
            assert!(
                result.is_err(),
                "`refresh: {value}` must still be rejected as unknown"
            );
        }
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
    fn test_assert_monotonic_defaults_false() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
        "#;
        let config: TimeseriesConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.assert_monotonic);
    }

    #[test]
    fn test_assert_monotonic_deserialization() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
            assert_monotonic: true
        "#;
        let config: TimeseriesConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.assert_monotonic);
    }

    #[test]
    fn test_assert_monotonic_rejects_non_bool() {
        let yaml = r#"
            event_time_column: ts
            partition_column: dt
            granularity: day
            assert_monotonic: "yes please"
        "#;
        let result: Result<TimeseriesConfig, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "a non-boolean assert_monotonic value must be a configuration error, not a silent default"
        );
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
            unique_key: []
        "#;
        let config: PartitionGrainConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.safety_overrides,
            PartitionGrainSafetyOverrides::default()
        );
        assert!(!config.safety_overrides.allow_window_functions);
    }

    #[test]
    fn test_unique_key_defaults_empty() {
        let yaml = r#"
            safety_overrides: {}
        "#;
        let config: PartitionGrainConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.unique_key.is_empty());
    }

    #[test]
    fn test_nondeterministic_columns_retired_absent_parses_clean() {
        let yaml = r#"
            safety_overrides: {}
        "#;
        let config: PartitionGrainConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.nondeterministic_columns_retired, ());
    }

    /// `models.<name>.batched.nondeterministic_columns` in `smelt.yml` fails
    /// deserialization with a fix-it naming `columns.<c>.contract: plausible`
    /// and the `.sql`-frontmatter-only location, regardless of the value
    /// declared (`docs/specs/models.md` §"Constraint violations").
    #[test]
    fn test_smelt_yml_batched_nondeterministic_columns_is_refused_with_fixit() {
        let yaml = r#"
            nondeterministic_columns: [inserted_at, batch_id]
        "#;
        let err = serde_yaml::from_str::<PartitionGrainConfig>(yaml)
            .expect_err("smelt.yml nondeterministic_columns must be refused");
        let message = err.to_string();
        assert!(
            message.contains("columns.inserted_at.contract: plausible"),
            "fix-it must name columns.inserted_at.contract: plausible; got: {message}"
        );
        assert!(
            message.contains("columns.batch_id.contract: plausible"),
            "fix-it must name columns.batch_id.contract: plausible; got: {message}"
        );
        assert!(
            message.contains(".sql frontmatter"),
            "fix-it must point at the .sql frontmatter location; got: {message}"
        );
    }

    /// `models.<name>.batched: {unique_key: [a]}` in `smelt.yml` fails to
    /// parse, with the fix-it naming the top-level `merge_key: [a]`
    /// replacement (`docs/specs/models.md` §"Constraint violations").
    #[test]
    fn smelt_yml_batched_block_is_retired() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  x:
    batched:
      unique_key: [a]
"#;
        let err = serde_yaml::from_str::<Config>(yaml)
            .expect_err("smelt.yml `batched:` sub-block must be refused at parse time");
        let message = err.to_string();
        assert!(
            message.contains("merge_key"),
            "fix-it must name the top-level merge_key: replacement; got: {message}"
        );
        assert!(
            message.contains(r#"["a"]"#) || message.contains("[\"a\"]"),
            "fix-it must carry the caller's own value; got: {message}"
        );
    }

    /// A `batched:` block declaring `safety_overrides` and
    /// `nondeterministic_columns` names both replacements
    /// (`safety_overrides:` top-level, `columns.c.contract: plausible`); an
    /// empty `batched: {}` still errors with the generic fix-it naming all
    /// three replacement keys.
    #[test]
    fn smelt_yml_batched_block_names_every_declared_key() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  x:
    batched:
      safety_overrides:
        allow_having: true
      nondeterministic_columns: [c]
"#;
        let err = serde_yaml::from_str::<Config>(yaml)
            .expect_err("declared batched: sub-keys must be refused");
        let message = err.to_string();
        assert!(
            message.contains("top-level `safety_overrides:"),
            "fix-it must name the top-level safety_overrides: replacement; got: {message}"
        );
        assert!(
            message.contains("columns.c.contract: plausible"),
            "fix-it must name columns.c.contract: plausible; got: {message}"
        );

        let empty_yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  x:
    batched: {}
"#;
        let empty_err = serde_yaml::from_str::<Config>(empty_yaml)
            .expect_err("empty batched: {} must still be refused");
        let empty_message = empty_err.to_string();
        assert!(
            empty_message.contains("top-level `merge_key:")
                && empty_message.contains("top-level `safety_overrides:")
                && empty_message.contains("columns.c.contract: plausible"),
            "empty batched: {{}} must still name all three generic replacements; got: {empty_message}"
        );
    }

    /// `smelt.yml` `merge_key: [event_id]` on a `refresh: incremental` +
    /// `grain: partition` model surfaces as `get_incremental(...).unique_key`;
    /// the single-string sugar form is also accepted.
    #[test]
    fn merge_key_folds_into_incremental_config() {
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
    refresh: incremental
    grain: partition
    merge_key: [event_id]
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("merge_key: must parse");
        let incremental = config
            .get_incremental("daily_revenue")
            .expect("selected model returns Some(incremental)");
        assert_eq!(incremental.unique_key, vec!["event_id".to_string()]);

        // Single-string sugar form.
        let yaml_sugar = r#"
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
    refresh: incremental
    grain: partition
    merge_key: event_id
"#;
        let config: Config =
            serde_yaml::from_str(yaml_sugar).expect("single-string merge_key: must parse");
        let incremental = config
            .get_incremental("daily_revenue")
            .expect("selected model returns Some(incremental)");
        assert_eq!(incremental.unique_key, vec!["event_id".to_string()]);
    }

    /// Frontmatter's `merge_key:` wins wholesale over the `smelt.yml`
    /// model-override spelling when both set it, mirroring `unique_key:`'s
    /// precedence rule.
    #[test]
    fn merge_key_frontmatter_wins_over_smelt_yml() {
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
    refresh: incremental
    grain: partition
    merge_key: [from_yaml]
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("smelt.yml merge_key: must parse");
        let frontmatter_meta = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(Grain::Partition),
            batched: Some(PartitionGrainConfig {
                unique_key: vec!["from_frontmatter".to_string()],
                nondeterministic_columns_retired: (),
                safety_overrides: PartitionGrainSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let incremental = config
            .get_incremental_with_metadata("daily_revenue", Some(&frontmatter_meta))
            .expect("selected model returns Some(incremental)");
        assert_eq!(
            incremental.unique_key,
            vec!["from_frontmatter".to_string()],
            "frontmatter's merge_key: must win wholesale over the smelt.yml spelling"
        );
    }

    /// Declaring only `merge_key:` never confers identity: the model still
    /// derives `Grain::Partition` (not the composed key+time shape), and
    /// `get_unique_key_with_metadata` stays empty.
    #[test]
    fn merge_key_does_not_confer_identity() {
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
    refresh: incremental
    grain: partition
    merge_key: [event_id]
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("merge_key: must parse");
        assert_eq!(
            config.get_grain("daily_revenue"),
            Some(Grain::Partition),
            "merge_key: alone must not flip the declared grain"
        );
        assert_eq!(
            config.get_unique_key_with_metadata("daily_revenue", None),
            None,
            "merge_key: must never surface as the identity fact"
        );
    }

    /// The ephemeral/view "cannot have incremental configuration" checks
    /// that used to key off `ModelConfig::batched` presence keep firing
    /// off `merge_key:` (and `safety_overrides:`), the new top-level
    /// signal.
    #[test]
    fn ephemeral_model_with_merge_key_is_refused() {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: HashMap::new(),
            default_materialization: Materialization::View,
            models: {
                let mut models = HashMap::new();
                models.insert(
                    "my_model".to_string(),
                    ModelConfig {
                        materialization: Some(Materialization::Ephemeral),
                        timeseries: None,
                        refresh: None,
                        grain: None,
                        unique_key: None,
                        safety_overrides: None,
                        batched_retired: (),
                        merge_key: Some(vec!["a".to_string()]),
                        tags: vec![],
                        target: None,
                        format: None,
                    },
                );
                models
            },
            python: None,
            target: None,
            state: StateConfig::default(),
            maintenance: None,
            probes: ProbesConfig::default(),
        };

        let errors = config.validate_model_configs(&HashMap::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("incremental"));
    }

    #[test]
    fn test_functional_dependency_deserialization() {
        let yaml = r#"
            key: [customer_id]
            determines: customer_region
        "#;
        let fd: FunctionalDependency = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(fd.key, vec!["customer_id".to_string()]);
        assert_eq!(fd.determines, "customer_region");
    }

    #[test]
    fn test_functional_dependency_rejects_unknown_fields() {
        let yaml = r#"
            key: [customer_id]
            determines: customer_region
            narrows: true
        "#;
        assert!(serde_yaml::from_str::<FunctionalDependency>(yaml).is_err());
    }

    #[test]
    fn test_bounded_domain_deserialization() {
        let yaml = r#"
            column: category
            max_cardinality: 10000
        "#;
        let bd: BoundedDomain = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(bd.column, "category");
        assert_eq!(bd.max_cardinality, 10000);
    }

    /// An absent `max_cardinality` is a fail-loud parse error, not a
    /// permissive default — the field carries no `#[serde(default)]`.
    #[test]
    fn test_bounded_domain_rejects_missing_cap() {
        let yaml = r#"
            column: category
        "#;
        assert!(serde_yaml::from_str::<BoundedDomain>(yaml).is_err());
    }

    #[test]
    fn test_bounded_domain_rejects_unknown_fields() {
        let yaml = r#"
            column: category
            max_cardinality: 10000
            narrows: true
        "#;
        assert!(serde_yaml::from_str::<BoundedDomain>(yaml).is_err());
    }

    #[test]
    fn test_unique_key_deserialization() {
        let yaml = r#"
            unique_key:
              - id
              - source
        "#;
        let config: PartitionGrainConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.unique_key, vec!["id", "source"]);
    }

    #[test]
    fn test_incremental_strategy_serialization() {
        let strategy = IncrementalStrategy::DeleteInsert;
        let json = serde_json::to_string(&strategy).unwrap();
        assert_eq!(json, r#""delete_insert""#);
    }

    /// `merge` is not an incremental strategy — UPSERT is the physical
    /// primitive used by the keyed merge loop (`refresh: incremental` + `grain: key`),
    /// not a knob on `incremental:`. Deserialising it must fail. `append` and
    /// `insert_overwrite` are gone too — `IncrementalStrategy` has one variant,
    /// `DeleteInsert`; the backend trait's `insert_into_from_query`/
    /// `insert_overwrite` capability methods stay as the future admission point.
    #[test]
    fn incremental_strategy_append_and_insert_overwrite_are_gone() {
        for value in ["merge", "append", "insert_overwrite"] {
            let result: Result<IncrementalStrategy, _> =
                serde_json::from_str(&format!(r#""{value}""#));
            assert!(
                result.is_err(),
                "`{value}` must not deserialise as an IncrementalStrategy — `DeleteInsert` is \
                 the only variant"
            );
        }
    }

    #[test]
    fn retain_departed_parses_both_forms() {
        let cfg: ContractConfig = serde_yaml::from_str("retain_departed: true\n").unwrap();
        assert_eq!(cfg.retain_departed, Some(RetainDeparted::Bool(true)));

        let cfg: ContractConfig =
            serde_yaml::from_str("retain_departed:\n  tombstone: is_departed\n").unwrap();
        assert_eq!(
            cfg.retain_departed,
            Some(RetainDeparted::Tombstone {
                tombstone: "is_departed".to_string()
            })
        );

        let cfg: ContractConfig = serde_yaml::from_str("{}\n").unwrap();
        assert_eq!(cfg.retain_departed, None);
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

    /// `horizon_ceiling:` on `ModelMetadata` reuses `DataLatency`'s existing
    /// fail-loud interval parser — this test confirms the field wiring
    /// surfaces both the happy path and the malformed-value error, not the
    /// grammar itself (already covered by `test_data_latency_parse`).
    #[test]
    fn test_model_metadata_horizon_ceiling_deserializes_via_data_latency() {
        let yaml = "horizon_ceiling: '30 days'\n";
        let metadata: crate::metadata::ModelMetadata = serde_yaml::from_str(yaml).unwrap();
        let ceiling = metadata
            .horizon_ceiling
            .expect("horizon_ceiling must deserialize");
        assert_eq!(ceiling.seconds, 30 * 86400);
    }

    #[test]
    fn test_model_metadata_horizon_ceiling_rejects_malformed_value() {
        let yaml = "horizon_ceiling: 'banana'\n";
        let result: Result<crate::metadata::ModelMetadata, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "a malformed horizon_ceiling must be a fail-loud parse error, not a silent default"
        );
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
    fn test_materialized_view_storage_value_rejected() {
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
        let err = serde_yaml::from_str::<Config>(yaml)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("refresh: materialized_view"),
            "expected migration hint pointing to `refresh: materialized_view`, got: {err}"
        );
    }

    #[test]
    fn test_default_materialization_rejects_materialized_view() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
default_materialization: materialized_view
"#;
        let err = serde_yaml::from_str::<Config>(yaml)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("refresh: materialized_view"),
            "expected migration hint pointing to `refresh: materialized_view`, got: {err}"
        );
    }

    #[test]
    fn test_materialization_storage_values_still_parse() {
        for (value, expected) in [
            ("table", Materialization::Table),
            ("view", Materialization::View),
            ("ephemeral", Materialization::Ephemeral),
        ] {
            let yaml = format!(
                r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  m:
    materialization: {value}
"#
            );
            let config: Config = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(
                config.models.get("m").unwrap().materialization,
                Some(expected)
            );
        }
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
            target: None,
            state: StateConfig::default(),
            maintenance: None,
            probes: ProbesConfig::default(),
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "my_model".to_string(),
            ModelMetadata {
                materialization: Some(Materialization::Ephemeral),
                batched: Some(PartitionGrainConfig {
                    unique_key: vec![],
                    nondeterministic_columns_retired: (),
                    safety_overrides: PartitionGrainSafetyOverrides::default(),
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
            target: None,
            state: StateConfig::default(),
            maintenance: None,
            probes: ProbesConfig::default(),
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
            target: None,
            state: StateConfig::default(),
            maintenance: None,
            probes: ProbesConfig::default(),
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "my_model".to_string(),
            ModelMetadata {
                materialization: Some(Materialization::Table),
                batched: Some(PartitionGrainConfig {
                    unique_key: vec![],
                    nondeterministic_columns_retired: (),
                    safety_overrides: PartitionGrainSafetyOverrides::default(),
                }),
                ..Default::default()
            },
        );

        let errors = config.validate_model_configs(&metadata);
        assert!(errors.is_empty());
    }

    /// BUG-056: `event_time_column`/`partition_column`/`granularity` are fields
    /// on `timeseries:`, not `batched:`. Because `PartitionGrainConfig` uses
    /// `deny_unknown_fields`, putting them under `batched:` must fail at
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
    refresh: incremental
    grain: partition
    batched:
      event_time_column: ts
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "event_time_column under batched: must fail — belongs under timeseries:"
        );
    }

    /// BUG-056 regression: correct format has `timeseries:` and `merge_key:`
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
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
    merge_key: [transaction_id]
"#;
        let config: Config =
            serde_yaml::from_str(yaml).expect("timeseries + merge_key as siblings must parse");
        let model = config.models.get("daily_revenue").unwrap();
        let ts = model.timeseries.as_ref().unwrap();
        assert_eq!(ts.event_time_column, "transaction_timestamp");
        assert_eq!(ts.partition_column, "revenue_date");
        assert_eq!(ts.granularity, Granularity::Day);
        assert_eq!(model.merge_key, Some(vec!["transaction_id".to_string()]));
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
vars:
  env: dev
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

    /// D-04: `schema` is optional; omitting it must produce `"main"`.
    #[test]
    fn target_schema_defaults_to_main() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("target without schema must parse");
        assert_eq!(
            config.targets["dev"].schema, "main",
            "omitted schema must default to main"
        );
    }

    /// D-04: an explicit `schema` value must be preserved as-is.
    #[test]
    fn explicit_schema_is_honored() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: analytics
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("config must parse");
        assert_eq!(
            config.targets["dev"].schema, "analytics",
            "explicit schema must be preserved"
        );
    }

    /// `settings:` map on a DuckDB target parses from YAML and round-trips.
    #[test]
    fn target_settings_parses_from_yaml() {
        let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
    settings:
      memory_limit: "1GB"
      threads: "2"
      temp_directory: /tmp/duckdb
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("dev").unwrap();
        let settings = target.settings.as_ref().expect("settings must be present");
        assert_eq!(
            settings.get("memory_limit").map(String::as_str),
            Some("1GB")
        );
        assert_eq!(settings.get("threads").map(String::as_str), Some("2"));
        assert_eq!(
            settings.get("temp_directory").map(String::as_str),
            Some("/tmp/duckdb")
        );
    }

    /// `settings:` is optional — absent from config means `None`.
    #[test]
    fn target_settings_defaults_to_none() {
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
        assert!(
            target.settings.is_none(),
            "settings must be None when absent"
        );
    }

    /// D-32: `format:` field on a model-config entry in `smelt.yml` parses correctly.
    #[test]
    fn model_config_format_parses_from_yaml() {
        let yaml = r#"
name: test
version: 1
targets:
  spark_prod:
    type: spark
    connect_url: sc://host:15002
    schema: prod
models:
  my_model:
    format: parquet
  other_model:
    format: delta
  no_format_model:
    materialization: table
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.models["my_model"].format, Some(TableFormat::Parquet),);
        assert_eq!(
            config.models["other_model"].format,
            Some(TableFormat::Delta),
        );
        assert_eq!(config.models["no_format_model"].format, None);
    }

    /// D-32: `get_format` tier 2 — model config overrides Spark target's delta default.
    #[test]
    fn get_format_model_config_overrides_target() {
        let yaml = r#"
name: test
version: 1
targets:
  spark_prod:
    type: spark
    connect_url: sc://host:15002
    schema: prod
models:
  my_model:
    format: parquet
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("spark_prod").unwrap();
        assert_eq!(
            config.get_format("my_model", None, target),
            Some(TableFormat::Parquet),
        );
    }

    /// D-32: `get_format` tier 1 — SQL frontmatter wins over model config.
    #[test]
    fn get_format_sql_metadata_beats_model_config() {
        let yaml = r#"
name: test
version: 1
targets:
  spark_prod:
    type: spark
    connect_url: sc://host:15002
    schema: prod
models:
  my_model:
    format: parquet
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("spark_prod").unwrap();
        let metadata = ModelMetadata {
            format: Some(TableFormat::Delta),
            ..Default::default()
        };
        assert_eq!(
            config.get_format("my_model", Some(&metadata), target),
            Some(TableFormat::Delta),
        );
    }

    /// D-32: DuckDB target always returns `None` — format is not applicable.
    #[test]
    fn get_format_duckdb_always_none() {
        let yaml = r#"
name: test
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  my_model:
    format: parquet
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let target = config.targets.get("dev").unwrap();
        assert_eq!(config.get_format("my_model", None, target), None);
    }

    /// D-32: no format at any tier → `None` for DuckDB, `Some(Delta)` for Spark.
    #[test]
    fn get_format_falls_through_to_target_default() {
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
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let duckdb = config.targets.get("dev").unwrap();
        let spark = config.targets.get("spark_prod").unwrap();
        assert_eq!(config.get_format("unknown", None, duckdb), None);
        assert_eq!(
            config.get_format("unknown", None, spark),
            Some(TableFormat::Delta),
        );
    }

    // ── D-33: default_materialization validation ─────────────────────────────

    fn minimal_config_yaml(default_mat: &str) -> String {
        format!(
            r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
default_materialization: {default_mat}
"#
        )
    }

    /// D-33: `default_materialization: test` is rejected at parse time
    /// with a hard error naming the forbidden value.
    #[test]
    fn default_materialization_test_is_rejected() {
        let yaml = minimal_config_yaml("test");
        let result = Config::parse_with_warnings(&yaml);
        assert!(
            result.is_err(),
            "`default_materialization: test` must be rejected, but parse_with_warnings returned Ok"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("test") || err.contains("Invalid"),
            "error must name the forbidden value 'test' or indicate an invalid value; got: {err}"
        );
    }

    /// `default_materialization: cumulative_aggregate` is rejected at parse time
    /// because `cumulative_aggregate` is no longer a valid materialization value —
    /// the Deserialize impl returns an unknown-value error before reaching the D-33 check.
    #[test]
    fn default_materialization_cumulative_aggregate_is_rejected() {
        let yaml = minimal_config_yaml("cumulative_aggregate");
        let result = Config::parse_with_warnings(&yaml);
        assert!(
            result.is_err(),
            "`default_materialization: cumulative_aggregate` must be rejected (unknown value)"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cumulative_aggregate") || err.contains("Invalid materialization"),
            "error must name the invalid value; got: {err}"
        );
    }

    /// D-33: `default_materialization: ephemeral` is permitted.
    #[test]
    fn default_materialization_ephemeral_is_allowed() {
        let yaml = minimal_config_yaml("ephemeral");
        let (config, _) = Config::parse_with_warnings(&yaml).expect("ephemeral is a legal default");
        assert_eq!(config.default_materialization, Materialization::Ephemeral);
    }

    /// D-33: table, view, ephemeral remain legal defaults (regression guard).
    /// `materialized_view` is no longer a storage-axis value — see
    /// `test_default_materialization_rejects_materialized_view`.
    #[test]
    fn default_materialization_standard_values_are_allowed() {
        for mat in ["table", "view", "ephemeral"] {
            let yaml = minimal_config_yaml(mat);
            assert!(
                Config::parse_with_warnings(&yaml).is_ok(),
                "`default_materialization: {mat}` must be accepted"
            );
        }
    }

    /// D-34: `state:` is a known top-level key — a smelt.yml with a `state:` block
    /// must not produce an unknown-key warning.
    #[test]
    fn state_key_does_not_warn() {
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
state:
  mode: stateless
"#;
        let (_config, warnings) = Config::parse_with_warnings(yaml).unwrap();
        let state_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("state")).collect();
        assert!(
            state_warnings.is_empty(),
            "`state:` must not produce an unknown-key warning, got: {:?}",
            state_warnings
        );
    }

    /// D-34: `vars:` still produces no warning (regression guard).
    #[test]
    fn vars_key_still_does_not_warn() {
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
vars:
  env: production
"#;
        let (_config, warnings) = Config::parse_with_warnings(yaml).unwrap();
        let vars_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("vars")).collect();
        assert!(
            vars_warnings.is_empty(),
            "`vars:` must not produce an unknown-key warning, got: {:?}",
            vars_warnings
        );
    }

    // ── D-47: StateMode posture lattice (P1 tests) ────────────────────────────

    #[test]
    fn state_mode_default_is_stateless() {
        let yaml = "name: p\nversion: 1\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.mode, StateMode::Stateless);
    }

    #[test]
    fn state_mode_environments_parses() {
        let yaml = "name: p\nversion: 1\nstate:\n  mode: environments\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.mode, StateMode::Environments);
    }

    #[test]
    fn state_mode_intervals_parses() {
        let yaml = "name: p\nversion: 1\nstate:\n  mode: intervals\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.mode, StateMode::Intervals);
    }

    #[test]
    fn state_mode_unknown_value_fails() {
        let yaml = "name: p\nversion: 1\nstate:\n  mode: bogus\n";
        let result = Config::parse_with_warnings(yaml);
        assert!(result.is_err(), "unknown mode must fail to parse");
    }

    #[test]
    fn state_mode_lattice_narrowing() {
        // environments can narrow to intervals or stateless
        assert!(StateMode::Environments.can_narrow_to(&StateMode::Intervals));
        assert!(StateMode::Environments.can_narrow_to(&StateMode::Stateless));
        assert!(StateMode::Environments.can_narrow_to(&StateMode::Environments));
        // intervals can narrow to stateless
        assert!(StateMode::Intervals.can_narrow_to(&StateMode::Stateless));
        assert!(StateMode::Intervals.can_narrow_to(&StateMode::Intervals));
        // stateless cannot widen to intervals or environments
        assert!(!StateMode::Stateless.can_narrow_to(&StateMode::Intervals));
        assert!(!StateMode::Stateless.can_narrow_to(&StateMode::Environments));
        // intervals cannot widen to environments
        assert!(!StateMode::Intervals.can_narrow_to(&StateMode::Environments));
    }

    #[test]
    fn warehouse_tables_defaults_to_allowed() {
        let yaml = "name: p\nversion: 1\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.warehouse_tables, WarehouseTables::Allowed);

        let yaml = "name: p\nversion: 1\nstate:\n  mode: intervals\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.warehouse_tables, WarehouseTables::Allowed);
    }

    #[test]
    fn warehouse_tables_none_parses() {
        let yaml = "name: p\nversion: 1\nstate:\n  warehouse_tables: none\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.state.warehouse_tables, WarehouseTables::None);
    }

    #[test]
    fn warehouse_tables_unknown_value_is_an_error() {
        let yaml = "name: p\nversion: 1\nstate:\n  warehouse_tables: sometimes\n";
        let result = Config::parse_with_warnings(yaml);
        assert!(
            result.is_err(),
            "unknown warehouse_tables must fail to parse"
        );
    }

    #[test]
    fn test_probes_defaults_to_per_run() {
        let yaml = "name: p\nversion: 1\n";
        let (config, _) = Config::parse_with_warnings(yaml).unwrap();
        assert_eq!(config.probes.cadence, ProbeCadence::PerRun);
    }

    #[test]
    fn test_probes_periodic_requires_positive_every_n_runs() {
        let missing_block = "name: p\nversion: 1\nprobes:\n  cadence: periodic\n";
        assert!(
            Config::parse_with_warnings(missing_block).is_err(),
            "periodic without a `periodic.every_n_runs` block must be a configuration error"
        );

        let zero =
            "name: p\nversion: 1\nprobes:\n  cadence: periodic\n  periodic:\n    every_n_runs: 0\n";
        assert!(
            Config::parse_with_warnings(zero).is_err(),
            "every_n_runs: 0 must be a configuration error, never a silent default"
        );

        let ok =
            "name: p\nversion: 1\nprobes:\n  cadence: periodic\n  periodic:\n    every_n_runs: 5\n";
        let (config, _) = Config::parse_with_warnings(ok).unwrap();
        assert_eq!(
            config.probes.cadence,
            ProbeCadence::Periodic { every_n_runs: 5 }
        );
    }

    #[test]
    fn test_probes_rejects_unknown_cadence_and_unknown_fields() {
        let unknown_cadence = "name: p\nversion: 1\nprobes:\n  cadence: sometimes\n";
        assert!(
            Config::parse_with_warnings(unknown_cadence).is_err(),
            "an unrecognised cadence value must fail loud, never fall back to a default"
        );

        let unknown_field = "name: p\nversion: 1\nprobes:\n  cadence: per_run\n  bogus: true\n";
        assert!(
            Config::parse_with_warnings(unknown_field).is_err(),
            "an unknown field under probes: must fail loud (deny_unknown_fields)"
        );
    }
}
