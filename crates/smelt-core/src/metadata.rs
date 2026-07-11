//! Metadata extraction from SQL files with YAML frontmatter.
//!
//! Supports two formats:
//! 1. Single-model files with YAML frontmatter:
//!    ```sql
//!    ---
//!    name: daily_revenue
//!    materialization: table
//!    ---
//!    SELECT ...
//!    ```
//!
//! 2. Multi-model files with section delimiters:
//!    ```sql
//!    --- name: model1 ---
//!    materialization: table
//!    ---
//!    SELECT ...
//!
//!    --- name: model2 ---
//!    materialization: view
//!    ---
//!    SELECT ...
//!    ```

use crate::config::{
    BatchedConfig, DataLatency, Materialization, RefreshStrategy, StateConfig, TimeseriesConfig,
};
use crate::frontmatter::{parse_frontmatter, DeclarationKind};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use thiserror::Error;

/// A test constraint on a column.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum ColumnTest {
    /// Simple test like "not_null" or "unique"
    Simple(String),
    /// Parameterized test like {accepted_values: [a, b]} or {min: 0}
    Parameterized(BTreeMap<String, serde_yaml::Value>),
}

/// Frontmatter knobs for a `smelt.test` declaration.
/// The model under test, mocks, and expectations live in the grammar
/// (`AS (...)`, `PASSING`, `EXPECT`) — not here.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct TestConfig {
    /// Number of property-based test cases (default 10)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cases: Option<u32>,
    /// Whether to check row order (default false = set comparison)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_order: Option<bool>,
}

/// Severity of a `smelt.check` violation.
///
/// `error` (default): a failing check (non-zero rows returned) fails the run
/// and blocks downstream steps. `warn`: reported but does not fail/block.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckSeverity {
    /// A violation fails the check (nonzero exit; blocks downstream).
    #[default]
    Error,
    /// A violation is reported but does not fail/block.
    Warn,
}

/// Frontmatter knobs for a `smelt.check` declaration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct CheckConfig {
    /// Severity of a check violation (default: `error`).
    #[serde(default)]
    pub severity: CheckSeverity,
}

/// Schema evolution configuration for a model.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SchemaEvolutionConfig {
    /// Strategy for handling schema changes.
    /// `alter_and_backfill` (default): use ALTER TABLE + UPDATE when possible.
    /// `full_refresh`: always DROP + CREATE on schema changes.
    #[serde(default)]
    pub strategy: SchemaEvolutionStrategy,
}

/// How to handle schema changes during incremental runs.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub enum SchemaEvolutionStrategy {
    /// Use ALTER TABLE with DEFAULT values and UPDATE backfill when possible.
    #[default]
    #[serde(rename = "alter_and_backfill")]
    AlterAndBackfill,
    /// Always fall back to full refresh on any schema change.
    #[serde(rename = "full_refresh")]
    FullRefresh,
}

/// Per-column metadata declared in model frontmatter.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ColumnMetadata {
    /// How late data can arrive for this column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_latency: Option<DataLatency>,

    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Column-level test constraints (not_null, unique, accepted_values, min, max)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<ColumnTest>,

    /// Default SQL expression for schema evolution (used when adding NOT NULL columns
    /// via ALTER TABLE instead of full refresh).
    ///
    /// This is a raw SQL expression string, e.g. `"0"`, `"'unknown'"`, `"NULL"`,
    /// or complex type expressions like `"STRUCT_PACK(a := 0, b := '')"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// SQL expression for backfilling existing rows during schema evolution.
    /// Used in UPDATE statements after ALTER TABLE ADD COLUMN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backfill: Option<String>,

    /// The column's equivalence contract (default `exact`). `plausible`
    /// admits non-determinism in a payload column; barred from every
    /// skeleton position, with cross-model fail-loud propagation. See
    /// `docs/specs/models.md` §"`columns:` — column metadata" and
    /// `docs/specs/maintenance_plan.md` §Surface "The plan (derived,
    /// reported)" (the guarantee ledger's equivalence-contract axis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<Contract>,
}

/// A column's declared equivalence contract (`columns.<c>.contract`).
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Contract {
    /// Bit-preserving across a technique change at a fixed processed-input
    /// set (the default).
    #[default]
    Exact,
    /// Admits non-determinism in a payload column. Barred from every
    /// skeleton position (identity/grouping/dedup/ordering).
    Plausible,
}

/// Author override hatches for virtual-environment reuse (D-46).
///
/// Declared in SQL frontmatter under the `reuse:` key.
/// `deny_unknown_fields` ensures unrecognised sub-keys produce a parse error.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReuseConfig {
    /// Reuse the existing materialization across an output-preserving change
    /// rather than re-rolling the dice (non-deterministic models only).
    #[serde(default)]
    pub accept_current: bool,
    /// Assert this model is deterministic-in-practice; the prover trusts it
    /// and logs the assertion.
    #[serde(default)]
    pub assert_deterministic: bool,
}

/// Metadata for a single model extracted from frontmatter
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelMetadata {
    /// Model name (optional in single-model files, required in multi-model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Generator directive: `"models"` marks the file as a multi-model
    /// generator file whose body is a `List<ModelDef>` meta expression.
    /// Any value other than `"models"` is rejected at parse time with
    /// `MetadataError::GeneratesUnknownValue`. Mutually exclusive with
    /// `name:` and with Layer-1 `--- name: foo ---` section delimiters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generates: Option<String>,

    /// Materialization strategy (table or view)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization: Option<Materialization>,

    /// Time-dimension declaration (event_time_column, partition_column, granularity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeseries: Option<TimeseriesConfig>,

    /// `batched:` block — optional configuration (`unique_key`,
    /// `safety_overrides`) layered on top of the `refresh: batched` selector.
    /// Selection itself is `refresh: batched` (`refresh` field below), not the
    /// presence of this block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batched: Option<BatchedConfig>,

    /// Target to execute this model on (overrides smelt.yml and CLI --target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,

    /// Tags for organization/filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Model owner (team/person)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Per-column metadata (latency, description, etc.)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub columns: HashMap<String, ColumnMetadata>,

    /// Backend-specific hints (forward compatibility)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub backend_hints: HashMap<String, serde_yaml::Value>,

    /// Test configuration (frontmatter knobs for `smelt.test` declarations,
    /// e.g. `cases` and `check_order`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestConfig>,

    /// Check configuration (frontmatter knobs for `smelt.check` declarations,
    /// e.g. `severity`). Not deserialized directly by serde (the `severity`
    /// key is top-level in the YAML); populated in `extract_single_model` by
    /// detecting the declaration kind from the body and extracting `severity`
    /// from the validated frontmatter map.
    #[serde(skip)]
    pub check: Option<CheckConfig>,

    /// Schema evolution configuration (opt out with strategy: full_refresh)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_evolution: Option<SchemaEvolutionConfig>,

    /// Override table format for this model (e.g., parquet for a specific model on a Delta target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<crate::config::TableFormat>,

    /// Virtual-environment reuse override hatches (D-46).
    /// `accept_current` and `assert_deterministic` are explicit, logged
    /// user overrides of the reuse prover's default verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse: Option<ReuseConfig>,

    /// Stateful model: apply a breaking change in place, accept no clean revert.
    /// Escape hatch for tables too large to rebuild (D-46).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forward_only: bool,

    /// Per-model state posture (D-47). When set, the model narrows (not widens)
    /// the project's `state.mode`. A model narrowed to `stateless` opts out of
    /// snapshot reuse entirely even when the project is `environments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateConfig>,

    /// Refresh axis: how stored output is recomputed across runs.
    ///
    /// `None` / `Some(Full)` — default full rebuild from scratch.
    /// `Some(Incremental)` — smelt runs the derived maintenance plan each
    /// run; requires a sibling `grain:` (see [`ModelMetadata::grain`]).
    /// Opt-in: `materialization: table` + `refresh: incremental` +
    /// `grain: key` (the former `refresh: keyed`).
    ///
    /// See `docs/specs/models.md` §"Refresh axis" and
    /// `docs/specs/keyed_models.md` §Surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshStrategy>,

    /// Declared grain (`partition` | `key` | `key_per_partition`) — the
    /// output shape and row identity for a `refresh: incremental` model.
    /// Required whenever `refresh: incremental` is set; rejected (hard
    /// error, see [`validate_timeseries`]) otherwise. See
    /// `docs/specs/models.md` §"Refresh axis".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<crate::config::Grain>,

    /// Model-scoped functional-dependency declarations (`key → determines`).
    /// See `crate::config::FunctionalDependency` and `model_properties.md`
    /// §"Model-scoped declarations".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functional_dependencies: Vec<crate::config::FunctionalDependency>,

    /// Model-scoped bounded-domain / space-budget declaration
    /// (`column` + required `max_cardinality`). See
    /// `crate::config::BoundedDomain` and `model_properties.md`
    /// §"Model-scoped declarations". A model asserts at most one bounded
    /// domain today (one holistic-aggregate column per model); the field is
    /// `Option`, not a list — an absent cap is a YAML parse error, never a
    /// silent default (no `#[serde(default)]` on `BoundedDomain::max_cardinality`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounded_domain: Option<crate::config::BoundedDomain>,

    /// Model-scoped horizon-ceiling declaration — the modeller's warning
    /// ceiling on the maintained window. The horizon is always **derived**
    /// from the model's own reach (lookback, window frames, join
    /// contribution); this declaration never relaxes or narrows the derived
    /// clamp. It only licenses a compile-time warning when the derived
    /// horizon would exceed the declared ceiling. See
    /// `docs/specs/maintenance_plan.md` §"Windowed maintenance and the
    /// horizon". Reuses `crate::config::DataLatency`'s existing fail-loud
    /// interval parser — no new interval grammar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon_ceiling: Option<crate::config::DataLatency>,

    /// Per-cell technique preferences/pins and the scan-locality guardrail
    /// (`defaults.prefer`, `cells[]`, `scan_bounds`). See
    /// `docs/specs/maintenance_plan.md` §Surface "Frontmatter".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<crate::config::MaintenanceConfig>,
}

impl ModelMetadata {
    /// Returns `true` when this model uses the keyed merge loop.
    ///
    /// The opt-in is `materialization: table` + `refresh: incremental` +
    /// `grain: key` (the former `refresh: keyed`).
    /// Every keyed detection site must route through this predicate.
    pub fn is_keyed(&self) -> bool {
        self.refresh == Some(RefreshStrategy::Incremental)
            && self.grain == Some(crate::config::Grain::Key)
    }

    /// Returns `true` when this model uses the partition-grain window-forward
    /// refresh loop.
    ///
    /// The opt-in is `materialization: table` + `refresh: incremental` +
    /// `grain: partition` (the former `refresh: batched`).
    /// Every partition-grain detection site must route through this predicate.
    pub fn is_partition_grain(&self) -> bool {
        self.refresh == Some(RefreshStrategy::Incremental)
            && self.grain == Some(crate::config::Grain::Partition)
    }

    /// Returns `true` when this model delegates freshness to a backend's
    /// native incremental-view maintenance.
    ///
    /// The opt-in is `materialization: table` + `refresh: materialized_view`.
    /// Every materialized-view detection site must route through this predicate.
    pub fn is_materialized_view(&self) -> bool {
        self.refresh == Some(RefreshStrategy::MaterializedView)
    }
}

/// Complete file metadata (single or multi-model)
#[derive(Debug, Clone, PartialEq)]
pub enum FileMetadata {
    /// File has no frontmatter
    Empty,

    /// Single model with frontmatter
    Single {
        metadata: Box<ModelMetadata>,
        /// Byte offset where SQL starts (after closing ---)
        sql_offset: usize,
    },

    /// Multiple models in one file (Layer-1 section delimiter format)
    Multi { models: Vec<ModelSection> },

    /// Generator file: `generates: models` frontmatter marks the body as a
    /// meta-evaluable `List<ModelDef>` expression. Each emitted `ModelDef`
    /// value becomes a model in the workspace.
    Generator {
        /// The parsed frontmatter (includes `generates: "models"` and any
        /// other allowed keys such as `tags`, `owner`, etc.).
        metadata: Box<ModelMetadata>,
        /// Byte offset of the first character of the body (the byte
        /// immediately after the closing `---\n` line of the frontmatter).
        body_offset: usize,
    },
}

/// One model section in a multi-model file
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSection {
    pub metadata: ModelMetadata,
    /// Byte range of SQL in file
    pub sql_range: Range<usize>,
}

/// Which mutual-exclusivity rule was violated in a `generates: models` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedKind {
    /// The frontmatter contained a `name:` field alongside `generates: models`.
    NameField,
    /// The body contained at least one Layer-1 `--- name: foo ---` section
    /// delimiter alongside `generates: models` frontmatter.
    SectionDelimiter,
}

/// A 1-based line + column position within the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}

/// Errors that can occur during metadata extraction
#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("YAML parse error: {0}")]
    YamlParseError(#[from] serde_yaml::Error),

    #[error("Missing model name in multi-model file at section {0}")]
    MissingModelName(usize),

    #[error("Malformed section delimiter at line {0}: expected '--- name: model_name ---'")]
    MalformedDelimiter(usize),

    #[error("Frontmatter not closed: missing closing '---' after line {0}")]
    UnclosedFrontmatter(usize),

    /// `generates:` value other than `"models"` was supplied.
    /// `value_span` is the 1-based line/column of the value token in the YAML.
    #[error("generates must be `models`; found {value}")]
    GeneratesUnknownValue {
        value: String,
        value_span: SourceSpan,
    },

    /// `generates: models` combined with a `name:` frontmatter field or
    /// with Layer-1 `--- name: foo ---` section delimiters.
    #[error("generates: models cannot coexist with bare-model identity (name field or section delimiter)")]
    GeneratesMixedWithBareModel {
        offending: MixedKind,
        span: SourceSpan,
    },

    /// A model declares `refresh: batched` without a sibling `timeseries:` block.
    #[error("TimeseriesRequiredForBatched: model declares `refresh: batched` but has no `timeseries:` block — add a `timeseries:` block with event_time_column, partition_column, and granularity")]
    TimeseriesRequiredForBatched,

    /// The `timeseries:` block violates a structural rule.
    #[error("MalformedTimeseries: {message}")]
    MalformedTimeseries { message: String },

    /// A model declares `refresh: keyed` and a `timeseries:` block without
    /// key temporal locality being established. Keyed outputs are not
    /// themselves timeseries by default — the rule reads the partition
    /// shape from the driving source instead (see
    /// `docs/specs/keyed_models.md` §"Output shape").
    #[error("KeyedForbidsTimeseries: keyed models must not declare a `timeseries:` block — the keyed output has no partition column; the rule reads the partition shape from the driving source")]
    KeyedForbidsTimeseries,

    /// A model declares `refresh: incremental` + `grain: key` (or another
    /// keyed-output mode) and a `batched:` block.
    #[error("KeyedForbidsBatched: keyed and partition-grain incremental are different refresh strategies with different equivalence contracts — pick one (see docs/specs/keyed_models.md)")]
    KeyedForbidsBatched,

    /// A model declares a `batched:` block without `refresh: incremental` +
    /// `grain: partition`.
    #[error("BatchedRequiresRefreshBatched: model declares a `batched:` block but is not `refresh: incremental` + `grain: partition` — add those keys or remove the `batched:` block")]
    BatchedRequiresRefreshBatched,

    /// A model declares `refresh: materialized_view` and a `timeseries:` block.
    /// Like `keyed`, the engine-maintained output is a keyed lookup with
    /// no partition column (`docs/specs/materialized_view.md` §"Constraints
    /// & Invariants").
    #[error("MaterializedViewForbidsTimeseries: refresh: materialized_view models must not declare a `timeseries:` block — the output is a keyed lookup with no partition column")]
    MaterializedViewForbidsTimeseries,

    /// A model declares `refresh: materialized_view` and a `batched:` block.
    /// The engine, not smelt, owns freshness for this mode — there is no
    /// smelt-driven batch loop to configure.
    #[error("MaterializedViewForbidsBatched: refresh: materialized_view models must not declare a `batched:` block — the engine owns freshness for this mode, not smelt's batch loop")]
    MaterializedViewForbidsBatched,

    /// A `functional_dependencies:` entry is structurally invalid: an empty
    /// `key`/`determines`, a `determines` column also listed in `key`
    /// (self-contradictory), or a `key`/`determines` column absent from the
    /// model's SQL body.
    #[error("MalformedFunctionalDependency: {message}")]
    MalformedFunctionalDependency { message: String },

    /// A `bounded_domain:` declaration is structurally invalid: an absent
    /// (already caught at YAML-parse time by the required field) or
    /// non-positive `max_cardinality`, an empty `column`, or a `column`
    /// absent from the model's SQL body.
    #[error("MalformedBoundedDomain: {message}")]
    MalformedBoundedDomain { message: String },

    /// A model declares `refresh: incremental` without a sibling `grain:`
    /// declaration. `grain:` is required whenever `refresh: incremental` is
    /// set — it declares the output's row identity/shape
    /// (`docs/specs/models.md` §"Refresh axis").
    #[error("GrainRequiredForIncremental: model declares `refresh: incremental` but has no `grain:` — add `grain: partition`, `grain: key`, or `grain: key_per_partition`")]
    GrainRequiredForIncremental,

    /// A model declares `grain:` without `refresh: incremental`. `grain:` is
    /// only meaningful alongside `refresh: incremental`
    /// (`docs/specs/models.md` §"Refresh axis").
    #[error("GrainRequiresIncremental: model declares `grain:` but is not `refresh: incremental` — add `refresh: incremental` or remove the `grain:` key")]
    GrainRequiresIncremental,
}

/// Check whether a `---\n`-prefixed source contains a `generates:` key in its
/// frontmatter block (the text between the opening `---` and the first closing
/// `---`). Returns `true` only when the key appears inside the frontmatter —
/// not inside the body.
fn frontmatter_has_generates(source: &str) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    // Skip first line ("---"), scan until closing "---" or EOF.
    for line in lines.iter().skip(1) {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("generates:") || trimmed == "generates:" {
            return true;
        }
    }
    false
}

/// Validate `refresh:`/`grain:`, `timeseries:`, and `batched:` constraints on
/// parsed metadata.
///
/// Pure function — operates only on the already-parsed `ModelMetadata` and the
/// SQL body text (for partition-column projection checks). Emits the first
/// constraint violation found, or `Ok(())` when all constraints pass.
///
/// Rules checked (per `models.md` §"Constraint violations", `timeseries.md` §Semantics):
/// - `refresh: incremental` without `grain:` → `GrainRequiredForIncremental`
/// - `grain:` without `refresh: incremental` → `GrainRequiresIncremental`
/// - `refresh: incremental` + `grain: partition` without `timeseries:` → `TimeseriesRequiredForBatched`
/// - `batched:` block without `refresh: incremental` + `grain: partition` → `BatchedRequiresRefreshBatched`
/// - `timeseries:` on `materialization: ephemeral` or `test` → `MalformedTimeseries`
/// - Legacy nested form (`event_time_column` inside `batched:`) was removed;
///   its presence in the YAML block now produces a YAML parse error (unknown field)
///   rather than a custom diagnostic, because `BatchedConfig` no longer
///   declares those fields.
/// - `partition_column` absent from the SQL body SELECT aliases → `MalformedTimeseries`
pub fn validate_timeseries(metadata: &ModelMetadata, sql_body: &str) -> Result<(), MetadataError> {
    use crate::config::Materialization;

    // Rule: grain: requires refresh: incremental, and vice versa — the two
    // keys are declared together (`docs/specs/models.md` §"Refresh axis").
    match (&metadata.refresh, &metadata.grain) {
        (Some(RefreshStrategy::Incremental), None) => {
            return Err(MetadataError::GrainRequiredForIncremental);
        }
        (refresh, Some(_)) if *refresh != Some(RefreshStrategy::Incremental) => {
            return Err(MetadataError::GrainRequiresIncremental);
        }
        _ => {}
    }

    // Rule: keyed forbids batched: — enforced here so the diagnostic
    // fires alongside the other materialization-block constraints.
    // Triggered by `refresh: incremental` + `grain: key`.
    if metadata.is_keyed() && metadata.batched.is_some() {
        return Err(MetadataError::KeyedForbidsBatched);
    }

    // Rule: keyed forbids timeseries: — the keyed output has no
    // partition column by default (key temporal locality establishment
    // is not yet built; see docs/specs/keyed_models.md §Known Divergences).
    if metadata.is_keyed() && metadata.timeseries.is_some() {
        return Err(MetadataError::KeyedForbidsTimeseries);
    }

    // Rule: materialized_view forbids batched: — the engine owns freshness
    // for this mode; there is no smelt-driven batch loop to configure.
    if metadata.is_materialized_view() && metadata.batched.is_some() {
        return Err(MetadataError::MaterializedViewForbidsBatched);
    }

    // Rule: materialized_view forbids timeseries: — like keyed, the
    // engine-maintained output is a keyed lookup with no partition column.
    if metadata.is_materialized_view() && metadata.timeseries.is_some() {
        return Err(MetadataError::MaterializedViewForbidsTimeseries);
    }

    // `refresh: incremental` + `grain: key` on a non-stored materialization:
    // - ephemeral: hard error — there is no persisted output to merge into.
    //   Mirrors the existing `ephemeral` + `incremental:` hard-error treatment.
    // - view: advisory warning only — config is ignored; mirrors `view + incremental`.
    if metadata.is_keyed() {
        if let Some(mat) = &metadata.materialization {
            match mat {
                Materialization::Ephemeral => {
                    return Err(MetadataError::MalformedTimeseries {
                        message: "ephemeral models cannot use refresh: incremental + \
                                  grain: key (ephemeral models have no persisted output \
                                  to merge into)"
                            .to_string(),
                    });
                }
                Materialization::View => {
                    tracing::warn!(
                        "model has `refresh: incremental` + `grain: key` but \
                         `materialization: view` — the refresh config is ignored for \
                         non-table materializations"
                    );
                    // Not an error — fall through.
                }
                _ => {}
            }
        }
    }

    // Rule: batched: block without refresh: incremental + grain: partition →
    // BatchedRequiresRefreshBatched
    if metadata.batched.is_some() && !metadata.is_partition_grain() {
        return Err(MetadataError::BatchedRequiresRefreshBatched);
    }

    // Rule: refresh: incremental + grain: partition without timeseries: →
    // TimeseriesRequiredForBatched
    if metadata.is_partition_grain() && metadata.timeseries.is_none() {
        return Err(MetadataError::TimeseriesRequiredForBatched);
    }

    let ts = match &metadata.timeseries {
        Some(t) => t,
        None => return Ok(()),
    };

    // Rule: timeseries: on ephemeral → MalformedTimeseries
    if let Some(mat) = &metadata.materialization {
        if mat == &Materialization::Ephemeral {
            return Err(MetadataError::MalformedTimeseries {
                message: "timeseries: is not allowed on ephemeral models (no persisted output)"
                    .to_string(),
            });
        }
    }

    // Rule: week_start requires granularity: week
    if ts.week_start.is_some() && ts.granularity != crate::config::Granularity::Week {
        return Err(MetadataError::MalformedTimeseries {
            message: "week_start requires granularity: week".to_string(),
        });
    }

    // Rule: week_start must be monday or sunday (spec timeseries.md §Surface)
    if let Some(ws) = &ts.week_start {
        use crate::config::Weekday;
        if !matches!(ws, Weekday::Monday | Weekday::Sunday) {
            return Err(MetadataError::MalformedTimeseries {
                message: format!(
                    "week_start must be 'monday' or 'sunday'; got '{}'",
                    serde_yaml::to_string(ws).unwrap_or_default().trim()
                ),
            });
        }
    }

    // Rule: partition_column must appear in the model's SELECT output (projection check)
    // Only check when there is actual SQL body content.
    if !sql_body.trim().is_empty() {
        let upper_body = sql_body.to_uppercase();
        let col_upper = ts.partition_column.to_uppercase();
        // Simple heuristic: the column name must appear somewhere in the SQL body.
        // A full SELECT-list parse is done by the planner; here we do a fast presence check
        // to catch the most obvious cases (completely absent column name).
        if !upper_body.contains(&col_upper) {
            return Err(MetadataError::MalformedTimeseries {
                message: format!(
                    "partition_column '{}' does not appear in the model's SQL body",
                    ts.partition_column
                ),
            });
        }
    }

    // Rule: `batched.nondeterministic_columns` must not name a column that
    // governs windowing, partition placement, or dedup identity — those
    // roles must stay deterministic regardless of any opt-in (Constraint 13;
    // `batched_models.md` §"Non-determinism and the equivalence contract").
    if let Some(batched) = &metadata.batched {
        for col in &batched.nondeterministic_columns {
            if col == &ts.event_time_column {
                return Err(MetadataError::MalformedTimeseries {
                    message: format!(
                        "nondeterministic_columns cannot list '{col}' — it is the \
                         event_time_column, which governs windowing and must stay \
                         deterministic"
                    ),
                });
            }
            if col == &ts.partition_column {
                return Err(MetadataError::MalformedTimeseries {
                    message: format!(
                        "nondeterministic_columns cannot list '{col}' — it is the \
                         partition_column, which governs partition placement and must \
                         stay deterministic"
                    ),
                });
            }
            if batched.unique_key.contains(col) {
                return Err(MetadataError::MalformedTimeseries {
                    message: format!(
                        "nondeterministic_columns cannot list '{col}' — it is a \
                         unique_key column, which governs dedup identity and must stay \
                         deterministic"
                    ),
                });
            }
        }
    }

    Ok(())
}

/// Validate `functional_dependencies:` entries on parsed metadata.
///
/// Pure function — operates only on the already-parsed `ModelMetadata` and the
/// SQL body text (for the column-presence check, mirroring
/// [`validate_timeseries`]'s partition-column heuristic). Emits the first
/// constraint violation found, or `Ok(())` when all entries pass (including
/// the common case of no `functional_dependencies:` at all).
///
/// Rules checked (`model_properties.md` §"Model-scoped declarations",
/// §Constraints "Declared escape hatches may only widen"):
/// - `key` must be non-empty and `determines` must be non-empty (a
///   self-contradictory / empty declaration is a configuration error).
/// - `determines` must not also appear in `key` (an FD cannot determine
///   itself — self-contradictory).
/// - Every `key` column and `determines` must appear in the model's SQL body
///   (a fast presence heuristic, same limitation as `validate_timeseries`'s
///   `partition_column` check — a full SELECT-list resolution is the
///   planner's job).
pub fn validate_functional_dependencies(
    metadata: &ModelMetadata,
    sql_body: &str,
) -> Result<(), MetadataError> {
    let upper_body = sql_body.to_uppercase();
    for fd in &metadata.functional_dependencies {
        if fd.key.is_empty() {
            return Err(MetadataError::MalformedFunctionalDependency {
                message: format!(
                    "functional dependency determining '{}' has an empty key — a functional \
                     dependency must name at least one key column",
                    fd.determines
                ),
            });
        }
        if fd.determines.trim().is_empty() {
            return Err(MetadataError::MalformedFunctionalDependency {
                message: "functional dependency has an empty `determines` column".to_string(),
            });
        }
        if fd.key.iter().any(|k| k == &fd.determines) {
            return Err(MetadataError::MalformedFunctionalDependency {
                message: format!(
                    "functional dependency is self-contradictory: '{}' cannot determine itself \
                     (it appears in both `key` and `determines`)",
                    fd.determines
                ),
            });
        }
        if !sql_body.trim().is_empty() {
            for col in fd.key.iter().chain(std::iter::once(&fd.determines)) {
                if !upper_body.contains(&col.to_uppercase()) {
                    return Err(MetadataError::MalformedFunctionalDependency {
                        message: format!(
                            "functional dependency names column '{col}' which does not appear \
                             in the model's SQL body"
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Validate the `bounded_domain:` declaration on parsed metadata.
///
/// Pure function — operates only on the already-parsed `ModelMetadata` and
/// the SQL body text (for the column-presence check, mirroring
/// [`validate_functional_dependencies`]'s heuristic). Emits the first
/// constraint violation found, or `Ok(())` when the declaration is absent or
/// passes every check.
///
/// Rules checked (`model_properties.md` §"Model-scoped declarations",
/// §Constraints "Declared escape hatches may only widen"):
/// - `max_cardinality` must be strictly positive — a zero-sized budget can
///   never license anything and is a configuration error, not treated as
///   "no declaration". (An *absent* cap is already a YAML parse error at the
///   `BoundedDomain` struct level — the field carries no
///   `#[serde(default)]` — so it never reaches this validator.)
/// - `column` must be non-empty.
/// - `column` must appear in the model's SQL body (a fast presence
///   heuristic, same limitation as `validate_functional_dependencies`'s
///   check — full SELECT-list resolution is the planner's job).
pub fn validate_bounded_domains(
    metadata: &ModelMetadata,
    sql_body: &str,
) -> Result<(), MetadataError> {
    let Some(bd) = metadata.bounded_domain.as_ref() else {
        return Ok(());
    };

    if bd.column.trim().is_empty() {
        return Err(MetadataError::MalformedBoundedDomain {
            message: "bounded_domain declaration has an empty `column`".to_string(),
        });
    }
    if bd.max_cardinality == 0 {
        return Err(MetadataError::MalformedBoundedDomain {
            message: format!(
                "bounded_domain declaration for column '{}' has max_cardinality: 0 — a \
                 zero-sized space budget can never license a holistic aggregate; declare a \
                 positive cap or remove the declaration",
                bd.column
            ),
        });
    }
    if !sql_body.trim().is_empty() && !sql_body.to_uppercase().contains(&bd.column.to_uppercase()) {
        return Err(MetadataError::MalformedBoundedDomain {
            message: format!(
                "bounded_domain declaration names column '{}' which does not appear in the \
                 model's SQL body",
                bd.column
            ),
        });
    }
    Ok(())
}

/// Extract metadata from SQL source text
///
/// Returns `FileMetadata::Empty` if no frontmatter is present (backward compatible).
pub fn extract_file_metadata(source: &str) -> Result<FileMetadata, MetadataError> {
    let trimmed = source.trim_start();

    // Check for single-model frontmatter
    if trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") {
        // Generator files: `generates:` in frontmatter takes priority over
        // `--- name:` section-delimiter detection. We must route to
        // `extract_single_model` (which then dispatches to `extract_generator`)
        // so the mutual-exclusivity guard fires before `extract_multi_model`
        // swallows the section delimiters in the body.
        if frontmatter_has_generates(trimmed) {
            return extract_single_model(trimmed);
        }
        // Check if this is actually a multi-model file (Layer-1 section delimiter format)
        if trimmed.contains("--- name:") {
            extract_multi_model(trimmed)
        } else {
            extract_single_model(trimmed)
        }
    }
    // Check for multi-model sections or malformed delimiters
    else if trimmed.contains("--- name:") {
        extract_multi_model(trimmed)
    }
    // Check for malformed delimiters that look like section markers
    else if let Some(line_num) = has_malformed_delimiter(source) {
        Err(MetadataError::MalformedDelimiter(line_num))
    }
    // No frontmatter - vanilla SQL
    else {
        Ok(FileMetadata::Empty)
    }
}

/// Extract the raw YAML text between the opening and closing `---` delimiters of
/// a single-model file's frontmatter, without parsing it.
///
/// Returns `None` if the file has no frontmatter. Used by `smelt-db` to call
/// `parse_frontmatter` for diagnostic purposes independently of
/// `extract_file_metadata`.
pub fn frontmatter_yaml_text(source: &str) -> Option<String> {
    let trimmed = source.trim_start();
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return None;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let closing = lines
        .iter()
        .skip(1)
        .position(|&line| line.trim() == "---")?
        + 1;
    Some(lines[1..closing].join("\n"))
}

/// Compute a 1-based `SourceSpan` for the YAML `generates:` value token given
/// the raw frontmatter YAML content (the text between the two `---` delimiters).
///
/// The outer file always starts with `---\n` (line 1); the YAML block begins on
/// line 2. We scan the YAML lines looking for `generates:` and return the
/// 1-based column position of the first non-whitespace character after the colon.
fn span_for_generates_value(yaml_content: &str) -> SourceSpan {
    // Outer file line 1 is "---"; YAML starts on line 2.
    let yaml_base_line = 2usize;
    for (idx, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("generates:") {
            let colon_rel = line.find(':').unwrap_or(0);
            let rest = &line[colon_rel + 1..];
            let value_col = colon_rel + 1 + rest.len() - rest.trim_start().len() + 1;
            return SourceSpan {
                line: yaml_base_line + idx,
                column: value_col,
            };
        }
    }
    SourceSpan { line: 1, column: 1 }
}

/// Compute a 1-based `SourceSpan` for the `name:` key inside the YAML block.
fn span_for_name_key(yaml_content: &str) -> SourceSpan {
    let yaml_base_line = 2usize;
    for (idx, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("name:") {
            let col = line.find("name").unwrap_or(0) + 1;
            return SourceSpan {
                line: yaml_base_line + idx,
                column: col,
            };
        }
    }
    SourceSpan { line: 1, column: 1 }
}

/// Extract a generator file: `generates: models` frontmatter with a body that
/// is a meta-evaluable `List<ModelDef>` expression.
///
/// Returns `Err(MetadataError::GeneratesUnknownValue)` when the `generates:`
/// value is not `"models"`, `Err(MetadataError::GeneratesMixedWithBareModel)`
/// when `name:` appears alongside `generates: models`, or
/// `Ok(FileMetadata::Generator { … })` on success.
fn extract_generator(
    source: &str,
    metadata: ModelMetadata,
    yaml_content: &str,
    closing_line_index: usize,
) -> Result<FileMetadata, MetadataError> {
    // Validate the `generates:` value.
    let value = metadata.generates.as_deref().unwrap_or("");
    if value != "models" {
        let value_span = span_for_generates_value(yaml_content);
        return Err(MetadataError::GeneratesUnknownValue {
            value: value.to_string(),
            value_span,
        });
    }

    // `name:` is mutually exclusive with `generates: models`.
    if metadata.name.is_some() {
        let span = span_for_name_key(yaml_content);
        return Err(MetadataError::GeneratesMixedWithBareModel {
            offending: MixedKind::NameField,
            span,
        });
    }

    // Calculate body_offset (byte just after the closing `---\n`).
    let body_offset = source
        .lines()
        .take(closing_line_index + 1)
        .map(|line| line.len() + 1) // +1 for newline
        .sum();

    // Check if the body contains Layer-1 section delimiters (`--- name: foo ---`).
    let body = &source[body_offset..];
    for (idx, line) in body.lines().enumerate() {
        if line.trim().starts_with("--- name:") {
            // Count actual file line number: closing_line_index+1 (0-based) + 1 (1-based) + body line offset.
            let body_line = closing_line_index + 2 + idx;
            return Err(MetadataError::GeneratesMixedWithBareModel {
                offending: MixedKind::SectionDelimiter,
                span: SourceSpan {
                    line: body_line,
                    column: 1,
                },
            });
        }
    }

    Ok(FileMetadata::Generator {
        metadata: Box::new(metadata),
        body_offset,
    })
}

/// Check if source contains lines that look like malformed section delimiters
///
/// Returns the 1-based line number of the first malformed delimiter, if any.
fn has_malformed_delimiter(source: &str) -> Option<usize> {
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        // Look for lines that start and end with --- but don't have "name:"
        if trimmed.starts_with("---") && trimmed.ends_with("---") && trimmed.len() > 6 {
            // Exclude exact "---" (valid delimiter)
            if trimmed != "---" && !trimmed.starts_with("--- name:") {
                return Some(idx + 1); // 1-based line number
            }
        }
    }
    None
}

/// Extract metadata from a single-model file (or a generator file that starts
/// with `---\n` frontmatter).
fn extract_single_model(source: &str) -> Result<FileMetadata, MetadataError> {
    let lines: Vec<&str> = source.lines().collect();

    if lines.is_empty() || lines[0] != "---" {
        return Ok(FileMetadata::Empty);
    }

    // Find closing ---
    let closing_line = lines
        .iter()
        .skip(1)
        .position(|&line| line.trim() == "---")
        .ok_or(MetadataError::UnclosedFrontmatter(1))?
        + 1; // +1 because we skipped first line

    // Extract YAML content between delimiters
    let yaml_lines = &lines[1..closing_line];
    let yaml_content = yaml_lines.join("\n");

    // Detect the declaration kind from the body text (everything after the
    // closing `---` line). Used to route `severity` through the catalogue for
    // check files and to populate `metadata.check`.
    let body_text: String = lines[(closing_line + 1)..].join("\n");
    let decl_kind = if body_text.contains("smelt.check") {
        DeclarationKind::Check
    } else {
        DeclarationKind::Model
    };

    // Route through the unified catalogue to filter unknown/inapplicable keys.
    // fm_diags are discarded here; smelt-db re-runs parse_frontmatter for the
    // diagnostic path so errors surface through file_diagnostics.
    let (validated_map, _fm_diags) = parse_frontmatter(&yaml_content, decl_kind);

    // For check files, extract `severity` from the validated map BEFORE serde
    // deserialization. The `severity` key is top-level in the YAML but is NOT
    // a serde field on `ModelMetadata`; we populate `metadata.check` from it.
    // For non-check files `validated_map` won't contain `severity` (it would
    // have been excluded by the catalogue as inapplicable).
    let check_config: Option<CheckConfig> = if decl_kind == DeclarationKind::Check {
        // Fail-loud: a present-but-invalid `severity` value (e.g. `severity: bogus`)
        // must surface as an error, never silently default to Error. An ABSENT key
        // defaults to Error (CheckSeverity::default()). Mirrors the reuse/state
        // strict-validation pattern below.
        let severity = match validated_map.get(serde_yaml::Value::String("severity".to_string())) {
            Some(v) => serde_yaml::from_value::<CheckSeverity>(v.clone())
                .map_err(MetadataError::YamlParseError)?,
            None => CheckSeverity::default(),
        };
        Some(CheckConfig { severity })
    } else {
        None
    };

    // Build a map suitable for ModelMetadata deserialization by removing the
    // check-only `severity` key (ModelMetadata has no `severity` field; serde
    // would ignore it with unknown-field tolerance, but we strip it for clarity).
    let mut model_map = validated_map.clone();
    model_map.remove(serde_yaml::Value::String("severity".to_string()));

    // Deserialize ModelMetadata from the validated (catalogue-filtered) map.
    // If a nested field (e.g. timeseries.granularity) fails to deserialize,
    // recover by stripping it so the model is still discovered. smelt-db emits
    // the MalformedTimeseries diagnostic via its own parse_frontmatter call.
    let mut metadata: ModelMetadata = if model_map.is_empty() {
        ModelMetadata::default()
    } else {
        // Pre-validate strict sub-fields before the resilient fallback path.
        // `reuse` uses deny_unknown_fields and `state` uses strict enum variants;
        // both must fail hard rather than be silently stripped (fail-loud discipline).
        // `materialization: cumulative_aggregate` and `materialization:
        // materialized_view` are also checked here to give a clear migration
        // error — `cumulative_aggregate` was removed (use `materialization:
        // table` + `refresh: incremental` + `grain: key` instead); `materialized_view`
        // was relocated from the storage axis to the refresh axis (use `refresh:
        // materialized_view` instead). `incremental:` is checked for the same
        // reason — the block was retired; use `refresh: incremental` + `grain: partition`
        // + `batched:` instead.
        for (key, value) in model_map.iter() {
            let key_str = key.as_str().unwrap_or("");
            if key_str == "reuse" {
                serde_yaml::from_value::<ReuseConfig>(value.clone())
                    .map_err(MetadataError::YamlParseError)?;
            } else if key_str == "state" {
                serde_yaml::from_value::<StateConfig>(value.clone())
                    .map_err(MetadataError::YamlParseError)?;
            // Fail hard specifically for the removed `cumulative_aggregate`
            // and relocated `materialized_view` values so the error is clear.
            // Other unknown materialization values use the resilient fallback
            // path below (surfaced as diagnostics by smelt-db rather than
            // hard-failing discovery).
            } else if key_str == "materialization"
                && matches!(
                    value.as_str(),
                    Some("cumulative_aggregate") | Some("materialized_view")
                )
            {
                return Err(MetadataError::YamlParseError(
                    serde_yaml::from_value::<Materialization>(value.clone()).unwrap_err(),
                ));
            } else if key_str == "incremental" {
                return Err(MetadataError::YamlParseError(serde_yaml::Error::custom(
                    "the `incremental:` block has been removed — use `refresh: incremental` + \
                     `grain: partition` + an optional `batched:` block instead (see \
                     docs/specs/batched_models.md)",
                )));
            // `refresh: cumulative` is a hard error pointing at the renamed
            // value, not a silently-stripped unknown value — the resilient
            // fallback below must not swallow this rename.
            } else if key_str == "refresh" {
                if let Err(e) =
                    serde_yaml::from_value::<crate::config::RefreshStrategy>(value.clone())
                {
                    return Err(MetadataError::YamlParseError(e));
                }
            }
        }

        match serde_yaml::from_value(serde_yaml::Value::Mapping(model_map.clone())) {
            Ok(m) => m,
            Err(_) => {
                // Strip known keys whose invalid values are surfaced as diagnostics
                // by smelt-db (MalformedTimeseries, etc.) so the model is still
                // discoverable.
                let mut fallback = model_map;
                fallback.remove(serde_yaml::Value::String("timeseries".to_string()));
                fallback.remove(serde_yaml::Value::String("batched".to_string()));
                fallback.remove(serde_yaml::Value::String("refresh".to_string()));
                fallback.remove(serde_yaml::Value::String("grain".to_string()));
                serde_yaml::from_value(serde_yaml::Value::Mapping(fallback)).unwrap_or_default()
            }
        }
    };

    // Populate the derived `check` config for check declarations.
    metadata.check = check_config;

    // Route generator files to `extract_generator`.
    if metadata.generates.is_some() {
        return extract_generator(source, metadata, &yaml_content, closing_line);
    }

    // Calculate SQL offset (after closing ---)
    let sql_offset = source
        .lines()
        .take(closing_line + 1)
        .map(|line| line.len() + 1) // +1 for newline
        .sum();

    Ok(FileMetadata::Single {
        metadata: Box::new(metadata),
        sql_offset,
    })
}

/// Extract metadata from a multi-model file
fn extract_multi_model(source: &str) -> Result<FileMetadata, MetadataError> {
    let mut models = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut current_line = 0;

    while current_line < lines.len() {
        // Skip empty lines and comments until next section
        while current_line < lines.len() {
            let line = lines[current_line].trim();
            if line.starts_with("--- name:") {
                break;
            }
            if !line.is_empty() && !line.starts_with("--") {
                // Found SQL without section delimiter - error
                return Err(MetadataError::MalformedDelimiter(current_line + 1));
            }
            current_line += 1;
        }

        if current_line >= lines.len() {
            break; // No more sections
        }

        // Parse section delimiter: "--- name: model_name ---"
        let delimiter_line = lines[current_line];
        let model_name = parse_section_delimiter(delimiter_line, current_line + 1)?;

        current_line += 1;

        // Find closing --- for this section's YAML
        let yaml_start_line = current_line;
        let closing_line = lines[current_line..]
            .iter()
            .position(|&line| line.trim() == "---")
            .ok_or(MetadataError::UnclosedFrontmatter(current_line + 1))?
            + current_line;

        // Extract YAML between delimiter and closing ---
        let yaml_lines = &lines[yaml_start_line..closing_line];
        let yaml_content = yaml_lines.join("\n");

        // Route through the unified catalogue to filter unknown/inapplicable keys.
        let mut metadata: ModelMetadata = if yaml_content.trim().is_empty() {
            ModelMetadata::default()
        } else {
            let (validated_map, _fm_diags) =
                parse_frontmatter(&yaml_content, DeclarationKind::Model);
            // Pre-validate strict sub-fields before the resilient fallback path.
            // `materialization: cumulative_aggregate` and `materialization:
            // materialized_view` are caught here for a clear migration error;
            // other unknown values follow the resilient path.
            for (key, value) in validated_map.iter() {
                let key_str = key.as_str().unwrap_or("");
                if key_str == "reuse" {
                    serde_yaml::from_value::<ReuseConfig>(value.clone())
                        .map_err(MetadataError::YamlParseError)?;
                } else if key_str == "state" {
                    serde_yaml::from_value::<StateConfig>(value.clone())
                        .map_err(MetadataError::YamlParseError)?;
                } else if key_str == "materialization"
                    && matches!(
                        value.as_str(),
                        Some("cumulative_aggregate") | Some("materialized_view")
                    )
                {
                    return Err(MetadataError::YamlParseError(
                        serde_yaml::from_value::<Materialization>(value.clone()).unwrap_err(),
                    ));
                } else if key_str == "incremental" {
                    return Err(MetadataError::YamlParseError(serde_yaml::Error::custom(
                        "the `incremental:` block has been removed — use `refresh: incremental` + \
                         `grain: partition` + an optional `batched:` block instead (see \
                         docs/specs/batched_models.md)",
                    )));
                }
            }

            match serde_yaml::from_value(serde_yaml::Value::Mapping(validated_map.clone())) {
                Ok(m) => m,
                Err(_) => {
                    let mut fallback = validated_map;
                    fallback.remove(serde_yaml::Value::String("timeseries".to_string()));
                    fallback.remove(serde_yaml::Value::String("batched".to_string()));
                    fallback.remove(serde_yaml::Value::String("refresh".to_string()));
                    fallback.remove(serde_yaml::Value::String("grain".to_string()));
                    serde_yaml::from_value(serde_yaml::Value::Mapping(fallback)).unwrap_or_default()
                }
            }
        };

        // Set model name from delimiter
        metadata.name = Some(model_name);

        current_line = closing_line + 1;

        // Find SQL range (from after closing --- to next section or EOF).
        // Guard against files without a trailing newline: summing (len + 1)
        // for every line overcounts the last line by 1 when it has no \n,
        // producing an index past source.len().  Capping at source.len() is
        // always correct because `sql_start_byte` for a section that starts
        // at EOF must equal `source.len()` (empty SQL body).
        let sql_start_byte: usize = if current_line >= lines.len() {
            source.len()
        } else {
            source
                .lines()
                .take(current_line)
                .map(|line| line.len() + 1)
                .sum()
        };

        // Find next section delimiter or EOF
        let sql_end_line = lines[current_line..]
            .iter()
            .position(|&line| line.trim().starts_with("--- name:"))
            .map(|pos| current_line + pos)
            .unwrap_or(lines.len());

        let sql_end_byte: usize = if sql_end_line >= lines.len() {
            source.len()
        } else {
            source
                .lines()
                .take(sql_end_line)
                .map(|line| line.len() + 1)
                .sum()
        };

        models.push(ModelSection {
            metadata,
            sql_range: sql_start_byte..sql_end_byte,
        });

        current_line = sql_end_line;
    }

    if models.is_empty() {
        Ok(FileMetadata::Empty)
    } else {
        Ok(FileMetadata::Multi { models })
    }
}

/// Parse a section delimiter line to extract the model name
///
/// Expected format: "--- name: model_name ---"
fn parse_section_delimiter(line: &str, line_number: usize) -> Result<String, MetadataError> {
    let trimmed = line.trim();

    // Must start with "--- name:" and end with "---"
    if !trimmed.starts_with("--- name:") || !trimmed.ends_with("---") {
        return Err(MetadataError::MalformedDelimiter(line_number));
    }

    // Extract name between "--- name:" and final "---"
    let after_prefix = &trimmed[9..]; // Skip "--- name:"
    let name_part = &after_prefix[..after_prefix.len() - 3]; // Remove final "---"
    let name = name_part.trim();

    if name.is_empty() {
        return Err(MetadataError::MissingModelName(line_number));
    }

    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_frontmatter() {
        let source = "SELECT * FROM users";
        let result = extract_file_metadata(source).unwrap();
        assert_eq!(result, FileMetadata::Empty);
    }

    #[test]
    fn test_single_model_basic() {
        let source = r#"---
name: test_model
materialization: table
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("test_model".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_single_model_with_batched() {
        let source = r#"---
name: daily_revenue
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: day
tags: [revenue, core]
---
SELECT DATE(transaction_timestamp) as revenue_date, SUM(amount)
FROM transactions
GROUP BY 1"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("daily_revenue".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
                assert_eq!(metadata.tags, vec!["revenue", "core"]);
                assert_eq!(metadata.refresh, Some(RefreshStrategy::Incremental));
                assert_eq!(metadata.grain, Some(crate::config::Grain::Partition));

                let timeseries = metadata.timeseries.unwrap();
                assert_eq!(timeseries.event_time_column, "transaction_timestamp");
                assert_eq!(timeseries.partition_column, "revenue_date");
                assert_eq!(timeseries.granularity, crate::config::Granularity::Day);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_multi_model_file() {
        let source = r#"--- name: model1 ---
materialization: table
---
SELECT * FROM source1

--- name: model2 ---
materialization: view
---
SELECT * FROM source2"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Multi { models } => {
                assert_eq!(models.len(), 2);

                assert_eq!(models[0].metadata.name, Some("model1".to_string()));
                assert_eq!(
                    models[0].metadata.materialization,
                    Some(Materialization::Table)
                );

                assert_eq!(models[1].metadata.name, Some("model2".to_string()));
                assert_eq!(
                    models[1].metadata.materialization,
                    Some(Materialization::View)
                );
            }
            _ => panic!("Expected Multi variant"),
        }
    }

    #[test]
    fn test_invalid_yaml() {
        // `materialization: invalid_value` is a bad enum value — the catalogue
        // passes `materialization` (valid model key) but serde fails on the
        // value. The recovery path strips unknown nested failures and returns
        // Ok with partial metadata. Discovery stays resilient; smelt-db emits
        // the MalformedTimeseries diagnostic.
        let source = r#"---
name: test
materialization: invalid_value
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source);
        assert!(
            result.is_ok(),
            "discovery must be resilient to bad field values; got: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn test_unclosed_frontmatter() {
        let source = r#"---
name: test
materialization: table
SELECT * FROM users"#;

        let result = extract_file_metadata(source);
        assert!(matches!(result, Err(MetadataError::UnclosedFrontmatter(_))));
    }

    #[test]
    fn test_malformed_section_delimiter() {
        let source = r#"--- model1 ---
materialization: table
---
SELECT * FROM source1"#;

        let result = extract_file_metadata(source);
        assert!(matches!(result, Err(MetadataError::MalformedDelimiter(_))));
    }

    #[test]
    fn test_section_delimiter_parsing() {
        assert_eq!(
            parse_section_delimiter("--- name: my_model ---", 1).unwrap(),
            "my_model"
        );
        assert_eq!(
            parse_section_delimiter("--- name:  spaced_name  ---", 1).unwrap(),
            "spaced_name"
        );
        assert!(parse_section_delimiter("--- name: ---", 1).is_err()); // Empty name
        assert!(parse_section_delimiter("--- model_name ---", 1).is_err()); // Missing "name:"
    }

    #[test]
    fn test_backward_compatibility() {
        // Files without frontmatter should work
        let vanilla_sql = r#"
-- This is a comment
SELECT user_id, COUNT(*) as count
FROM events
GROUP BY user_id
"#;
        let result = extract_file_metadata(vanilla_sql).unwrap();
        assert_eq!(result, FileMetadata::Empty);
    }

    #[test]
    fn test_empty_frontmatter_in_multi_model() {
        let source = r#"--- name: simple_model ---
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Multi { models } => {
                assert_eq!(models.len(), 1);
                assert_eq!(models[0].metadata.name, Some("simple_model".to_string()));
                assert_eq!(models[0].metadata.materialization, None);
            }
            _ => panic!("Expected Multi variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_leading_whitespace() {
        // Python triple-quoted strings often produce leading newlines
        let source = "\n---\ntags:\n  - event_source\n---\nSELECT * FROM events";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.tags, vec!["event_source".to_string()]);
            }
            _ => panic!("Expected Single variant, got {:?}", result),
        }
    }

    #[test]
    fn test_unknown_field_rejected() {
        // `materialized` is a typo for `materialization` — it's unknown to the
        // catalogue, which filters it out and produces an Error diagnostic.
        // Discovery succeeds with partial metadata; smelt-db surfaces the error
        // as a FrontmatterParseError. The known field `name` is retained.
        let source = "---\nname: test\nmaterialized: table\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        assert!(
            result.is_ok(),
            "discovery must be resilient to unknown fields; got: {:?}",
            result.unwrap_err()
        );
        if let Ok(FileMetadata::Single { metadata, .. }) = result {
            assert_eq!(metadata.name, Some("test".to_string()));
            assert_eq!(
                metadata.materialization, None,
                "unknown key must be dropped"
            );
        }
    }

    #[test]
    fn test_unknown_field_incremental_key_rejected() {
        // `incremental_key` is unknown to the catalogue — filtered out.
        // Discovery succeeds; smelt-db emits FrontmatterParseError.
        let source = "---\nname: test\nincremental_key: user_id\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        assert!(
            result.is_ok(),
            "discovery must be resilient to unknown fields; got: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn test_frontmatter_with_leading_whitespace_and_spaces() {
        let source = "  \n\n---\nname: my_model\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("my_model".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant, got {:?}", result),
        }
    }

    #[test]
    fn test_frontmatter_with_target() {
        let source =
            "---\nname: my_model\ntarget: spark_prod\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.target, Some("spark_prod".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_without_target_is_none() {
        let source = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.target, None);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_ephemeral_materialization() {
        let source = "---\nname: staging\nmaterialization: ephemeral\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.materialization, Some(Materialization::Ephemeral));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_materialized_view_rejected() {
        let source = "---\nname: cached_report\nmaterialization: materialized_view\n---\nSELECT 1";

        let err = extract_file_metadata(source).unwrap_err().to_string();
        assert!(
            err.contains("refresh: materialized_view"),
            "expected migration hint pointing to `refresh: materialized_view`, got: {err}"
        );
    }

    #[test]
    fn test_frontmatter_with_column_default() {
        let source = r#"---
name: test_model
materialization: table
columns:
  status:
    default: "'unknown'"
  priority:
    default: "0"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let status = metadata.columns.get("status").unwrap();
                assert_eq!(status.default.as_ref().unwrap(), "'unknown'");
                let priority = metadata.columns.get("priority").unwrap();
                assert_eq!(priority.default.as_ref().unwrap(), "0");
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_complex_type_defaults() {
        let source = r#"---
name: test_model
materialization: table
columns:
  meta:
    default: "STRUCT_PACK(a := 0, b := '')"
  tags:
    default: "[]::VARCHAR[]"
  lookup:
    default: "MAP {}"
  scores:
    default: "ARRAY[1, 2, 3]"
  flag:
    default: "TRUE"
  nothing:
    default: "NULL"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let meta = metadata.columns.get("meta").unwrap();
                assert_eq!(
                    meta.default.as_ref().unwrap(),
                    "STRUCT_PACK(a := 0, b := '')"
                );
                let tags = metadata.columns.get("tags").unwrap();
                assert_eq!(tags.default.as_ref().unwrap(), "[]::VARCHAR[]");
                let lookup = metadata.columns.get("lookup").unwrap();
                assert_eq!(lookup.default.as_ref().unwrap(), "MAP {}");
                let scores = metadata.columns.get("scores").unwrap();
                assert_eq!(scores.default.as_ref().unwrap(), "ARRAY[1, 2, 3]");
                let flag = metadata.columns.get("flag").unwrap();
                assert_eq!(flag.default.as_ref().unwrap(), "TRUE");
                let nothing = metadata.columns.get("nothing").unwrap();
                assert_eq!(nothing.default.as_ref().unwrap(), "NULL");
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_schema_evolution() {
        let source = r#"---
name: test_model
materialization: table
schema_evolution:
  strategy: full_refresh
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(
                    metadata.schema_evolution.unwrap().strategy,
                    SchemaEvolutionStrategy::FullRefresh
                );
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_format_override() {
        let source = "---\nname: my_model\nmaterialization: table\nformat: parquet\n---\nSELECT 1";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.format, Some(crate::config::TableFormat::Parquet));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_without_format_is_none() {
        let source = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.format, None);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    // ── Generator file metadata tests ─────────────────────────────────────────

    /// A file with `generates: models` frontmatter routes to
    /// `FileMetadata::Generator` with correct `body_offset`.
    #[test]
    fn parse_generates_models_frontmatter_routes_to_generator_variant() {
        // Three-line frontmatter: "---\ngenerates: models\n---\n"
        // byte offsets: 0-3="---\n", 4-20="generates: models\n", 21-24="---\n"
        // body_offset should point to byte 25 (first byte after closing "---\n").
        let source = "---\ngenerates: models\n---\n[]\n";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Generator {
                metadata,
                body_offset,
            } => {
                assert_eq!(
                    metadata.generates.as_deref(),
                    Some("models"),
                    "metadata.generates must be Some(\"models\")"
                );
                // body_offset must point past the closing "---\n".
                assert_eq!(
                    &source[body_offset..],
                    "[]\n",
                    "body_offset must point to byte after closing ---\\n"
                );
            }
            other => panic!("Expected Generator variant, got: {:?}", other),
        }
    }

    /// `generates:` with a value other than `models` produces
    /// `MetadataError::GeneratesUnknownValue` carrying the offending value and a
    /// 1-based `(line, column)` span pointing at the first non-whitespace
    /// character after the `generates:` colon — the parser dispatch in Phase 2
    /// anchors the surfaced diagnostic at this span.
    #[test]
    fn parse_generates_unknown_value_emits_metadata_error() {
        let source = "---\ngenerates: views\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        match result {
            Err(MetadataError::GeneratesUnknownValue { value, value_span }) => {
                assert_eq!(value, "views");
                // Outer line 1 = "---"; YAML "generates: views" is line 2.
                // "generates:" occupies columns 1..=10; " views" begins at column 12.
                assert_eq!(
                    value_span,
                    SourceSpan {
                        line: 2,
                        column: 12,
                    },
                    "value_span should anchor at the first non-whitespace char after the colon"
                );
            }
            other => panic!("Expected GeneratesUnknownValue(views), got: {:?}", other),
        }
    }

    /// `generates: models` combined with a `name:` field produces
    /// `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::NameField, .. }`.
    #[test]
    fn parse_generates_with_name_field_emits_mixed_error() {
        let source = "---\ngenerates: models\nname: foo\n---\n[]";
        let result = extract_file_metadata(source);
        assert!(
            matches!(
                result,
                Err(MetadataError::GeneratesMixedWithBareModel {
                    offending: MixedKind::NameField,
                    ..
                })
            ),
            "Expected GeneratesMixedWithBareModel(NameField), got: {:?}",
            result
        );
    }

    /// `generates: models` combined with Layer-1 `--- name: foo ---` section
    /// delimiters produces `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::SectionDelimiter, .. }`.
    #[test]
    fn parse_generates_with_section_delimiter_emits_mixed_error() {
        let source = "---\ngenerates: models\n---\n--- name: foo ---\nSELECT 1\n--- name: bar ---\nSELECT 2\n";
        let result = extract_file_metadata(source);
        assert!(
            matches!(
                result,
                Err(MetadataError::GeneratesMixedWithBareModel {
                    offending: MixedKind::SectionDelimiter,
                    ..
                })
            ),
            "Expected GeneratesMixedWithBareModel(SectionDelimiter), got: {:?}",
            result
        );
    }

    /// Files without `generates:` frontmatter continue to parse to
    /// `Single`, `Multi`, or `Empty` (regression guard).
    #[test]
    fn parse_no_generates_keeps_single_or_multi_variants() {
        // Single variant.
        let single = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";
        assert!(
            matches!(
                extract_file_metadata(single).unwrap(),
                FileMetadata::Single { .. }
            ),
            "Non-generates file must parse to Single"
        );

        // Multi variant.
        let multi = "--- name: m1 ---\n---\nSELECT 1\n--- name: m2 ---\n---\nSELECT 2";
        assert!(
            matches!(
                extract_file_metadata(multi).unwrap(),
                FileMetadata::Multi { .. }
            ),
            "Non-generates file must parse to Multi"
        );

        // Empty variant.
        let empty = "SELECT * FROM foo";
        assert!(
            matches!(extract_file_metadata(empty).unwrap(), FileMetadata::Empty),
            "Non-frontmatter file must parse to Empty"
        );
    }

    /// A generator file with extra frontmatter keys (`tags`, `owner`) admits them.
    #[test]
    fn parse_generates_models_with_other_frontmatter_keys_admits_them() {
        let source = "---\ngenerates: models\ntags: [cohort]\nowner: data-team\n---\n[]";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Generator { metadata, .. } => {
                assert_eq!(metadata.tags, vec!["cohort".to_string()]);
                assert_eq!(metadata.owner.as_deref(), Some("data-team"));
            }
            other => panic!("Expected Generator variant, got: {:?}", other),
        }
    }

    // ── timeseries: block tests ───────────────────────────────────────────────

    /// A frontmatter with a `timeseries:` block parses to a `TimeseriesConfig`
    /// carrying the four fields.
    #[test]
    fn test_timeseries_block_parses() {
        let source = r#"---
materialization: table
timeseries:
  event_time_column: order_ts
  partition_column: order_date
  granularity: day
---
SELECT DATE_TRUNC('day', order_ts) AS order_date, order_ts
FROM smelt.orders_raw"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let ts = metadata.timeseries.expect("timeseries must be present");
                assert_eq!(ts.event_time_column, "order_ts");
                assert_eq!(ts.partition_column, "order_date");
                assert_eq!(ts.granularity, crate::config::Granularity::Day);
                assert_eq!(ts.week_start, None);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    /// A `.sql` file declaring `refresh: batched` with no `timeseries:` produces
    /// `TimeseriesRequiredForBatched` from `validate_timeseries`.
    #[test]
    fn test_batched_without_timeseries_errors() {
        // Build a ModelMetadata with refresh: batched but no timeseries
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Partition),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT event_date FROM foo")
            .expect_err("must error when refresh: batched has no timeseries:");
        assert!(
            matches!(err, MetadataError::TimeseriesRequiredForBatched),
            "Expected TimeseriesRequiredForBatched, got: {}",
            err
        );
    }

    /// A `batched:` block without `refresh: batched` is a hard error.
    #[test]
    fn test_batched_block_without_refresh_batched_errors() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            batched: Some(crate::config::BatchedConfig::default()),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
            .expect_err("must error when batched: has no refresh: batched");
        assert!(
            matches!(err, MetadataError::BatchedRequiresRefreshBatched),
            "Expected BatchedRequiresRefreshBatched, got: {}",
            err
        );
    }

    /// Listing `event_time_column` in `batched.nondeterministic_columns` is a
    /// configuration error (Constraint 13; `batched_models.md` §Surface) — that
    /// column governs windowing and can never tolerate non-determinism.
    #[test]
    fn test_nondeterministic_columns_rejects_event_time_column() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Partition),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "order_ts".to_string(),
                partition_column: "order_date".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec!["order_ts".to_string()],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT order_ts, order_date FROM foo")
            .expect_err("listing event_time_column in nondeterministic_columns must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(err.to_string().contains("order_ts"));
    }

    /// Listing `partition_column` in `batched.nondeterministic_columns` is a
    /// configuration error — that column governs partition placement.
    #[test]
    fn test_nondeterministic_columns_rejects_partition_column() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Partition),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "order_ts".to_string(),
                partition_column: "order_date".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec!["order_date".to_string()],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT order_ts, order_date FROM foo")
            .expect_err("listing partition_column in nondeterministic_columns must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(err.to_string().contains("order_date"));
    }

    /// Listing a `unique_key` column in `batched.nondeterministic_columns` is a
    /// configuration error — that column governs dedup identity.
    #[test]
    fn test_nondeterministic_columns_rejects_unique_key_column() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Partition),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "order_ts".to_string(),
                partition_column: "order_date".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec!["order_id".to_string()],
                nondeterministic_columns: vec!["order_id".to_string()],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT order_ts, order_date, order_id FROM foo")
            .expect_err("listing a unique_key column in nondeterministic_columns must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(err.to_string().contains("order_id"));
    }

    /// A payload column not overlapping event_time/partition/unique_key is a
    /// legitimate `nondeterministic_columns` entry and parses cleanly.
    #[test]
    fn test_nondeterministic_columns_accepts_payload_column() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Partition),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "order_ts".to_string(),
                partition_column: "order_date".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec!["order_id".to_string()],
                nondeterministic_columns: vec!["inserted_at".to_string()],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        validate_timeseries(
            &metadata,
            "SELECT order_ts, order_date, order_id, inserted_at FROM foo",
        )
        .expect("payload-only nondeterministic_columns must pass validation");
    }

    // ── functional_dependencies: validation (DC2) ────────────────────────────

    fn fd(key: &[&str], determines: &str) -> crate::config::FunctionalDependency {
        crate::config::FunctionalDependency {
            key: key.iter().map(|s| s.to_string()).collect(),
            determines: determines.to_string(),
        }
    }

    /// A valid FD naming columns present in the SQL body parses cleanly.
    #[test]
    fn test_functional_dependencies_accepts_valid_declaration() {
        let metadata = ModelMetadata {
            functional_dependencies: vec![fd(&["customer_id"], "customer_region")],
            ..Default::default()
        };
        validate_functional_dependencies(
            &metadata,
            "SELECT customer_id, customer_region FROM customers",
        )
        .expect("a valid functional dependency must pass validation");
    }

    /// An empty `key` is a self-contradictory declaration — fail-loud, not a
    /// silent default.
    #[test]
    fn test_functional_dependencies_rejects_empty_key() {
        let metadata = ModelMetadata {
            functional_dependencies: vec![fd(&[], "customer_region")],
            ..Default::default()
        };
        let err = validate_functional_dependencies(
            &metadata,
            "SELECT customer_id, customer_region FROM customers",
        )
        .expect_err("an empty key must be a configuration error");
        assert!(matches!(
            err,
            MetadataError::MalformedFunctionalDependency { .. }
        ));
    }

    /// A `determines` column also listed in `key` cannot determine itself —
    /// self-contradictory, refused.
    #[test]
    fn test_functional_dependencies_rejects_self_contradictory() {
        let metadata = ModelMetadata {
            functional_dependencies: vec![fd(
                &["customer_id", "customer_region"],
                "customer_region",
            )],
            ..Default::default()
        };
        let err = validate_functional_dependencies(
            &metadata,
            "SELECT customer_id, customer_region FROM customers",
        )
        .expect_err("determines column also in key must be a configuration error");
        assert!(matches!(
            err,
            MetadataError::MalformedFunctionalDependency { .. }
        ));
    }

    /// An FD naming a column absent from the model's SQL body is a
    /// configuration error (fail-loud, not silently accepted).
    #[test]
    fn test_functional_dependencies_rejects_absent_column() {
        let metadata = ModelMetadata {
            functional_dependencies: vec![fd(&["customer_id"], "phone_number")],
            ..Default::default()
        };
        let err = validate_functional_dependencies(
            &metadata,
            "SELECT customer_id, customer_region FROM customers",
        )
        .expect_err("a determines column absent from the SQL body must error");
        assert!(matches!(
            err,
            MetadataError::MalformedFunctionalDependency { .. }
        ));
        assert!(err.to_string().contains("phone_number"));
    }

    // ── bounded_domain: validation (DC3) ─────────────────────────────────────

    fn bounded_domain(column: &str, max_cardinality: u64) -> crate::config::BoundedDomain {
        crate::config::BoundedDomain {
            column: column.to_string(),
            max_cardinality,
        }
    }

    /// A valid bounded-domain declaration naming a column present in the SQL
    /// body, with a positive cap, parses cleanly.
    #[test]
    fn test_bounded_domain_accepts_valid_declaration() {
        let metadata = ModelMetadata {
            bounded_domain: Some(bounded_domain("category", 10_000)),
            ..Default::default()
        };
        validate_bounded_domains(&metadata, "SELECT category, amount FROM orders")
            .expect("a valid bounded-domain declaration must pass validation");
    }

    /// No declaration at all is the ordinary case — not an error.
    #[test]
    fn test_bounded_domain_absent_is_ok() {
        let metadata = ModelMetadata {
            bounded_domain: None,
            ..Default::default()
        };
        validate_bounded_domains(&metadata, "SELECT category, amount FROM orders")
            .expect("no bounded_domain declaration at all must not error");
    }

    /// A `max_cardinality: 0` cap can never license anything — fail-loud,
    /// not a silent default that behaves like "no declaration".
    #[test]
    fn test_bounded_domain_rejects_zero_cap() {
        let metadata = ModelMetadata {
            bounded_domain: Some(bounded_domain("category", 0)),
            ..Default::default()
        };
        let err = validate_bounded_domains(&metadata, "SELECT category, amount FROM orders")
            .expect_err("a zero max_cardinality must be a configuration error");
        assert!(matches!(err, MetadataError::MalformedBoundedDomain { .. }));
    }

    /// An empty `column` is a self-contradictory declaration — fail-loud.
    #[test]
    fn test_bounded_domain_rejects_empty_column() {
        let metadata = ModelMetadata {
            bounded_domain: Some(bounded_domain("", 10_000)),
            ..Default::default()
        };
        let err = validate_bounded_domains(&metadata, "SELECT category, amount FROM orders")
            .expect_err("an empty column must be a configuration error");
        assert!(matches!(err, MetadataError::MalformedBoundedDomain { .. }));
    }

    /// A `column` absent from the model's SQL body is a configuration error
    /// (fail-loud, not silently accepted).
    #[test]
    fn test_bounded_domain_rejects_absent_column() {
        let metadata = ModelMetadata {
            bounded_domain: Some(bounded_domain("region", 10_000)),
            ..Default::default()
        };
        let err = validate_bounded_domains(&metadata, "SELECT category, amount FROM orders")
            .expect_err("a column absent from the SQL body must error");
        assert!(matches!(err, MetadataError::MalformedBoundedDomain { .. }));
        assert!(err.to_string().contains("region"));
    }

    /// A `.sql` file declaring the retired `incremental:` block is a hard error
    /// naming `refresh: incremental` + `grain:` as the replacement (models.md hard-cut).
    #[test]
    fn test_incremental_block_is_hard_cut() {
        let source = r#"---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: ts
  partition_column: dt
  granularity: day
---
SELECT dt FROM foo"#;
        let err = extract_file_metadata(source)
            .expect_err("declaring the retired `incremental:` block must hard-error");
        let message = err.to_string();
        assert!(
            message.contains("refresh: incremental") && message.contains("grain:"),
            "error message must name refresh: incremental + grain: as the replacement; got: {}",
            message
        );
    }

    /// A `.sql` file declaring `event_time_column` inside `batched:` has a
    /// bad nested value (BatchedConfig has deny_unknown_fields). The recovery
    /// path strips `batched:` and returns Ok with partial metadata.
    /// Discovery is resilient; smelt-db surfaces a MalformedTimeseries diagnostic.
    #[test]
    fn test_legacy_nested_form_errors() {
        let source = r#"---
materialization: table
refresh: incremental
grain: partition
batched:
  event_time_column: ts
  partition_column: dt
  granularity: day
---
SELECT dt FROM foo"#;
        let result = extract_file_metadata(source);
        assert!(
            result.is_ok(),
            "discovery must be resilient to bad nested fields; got: {:?}",
            result.unwrap_err()
        );
        // The batched block is stripped in recovery; materialization is kept.
        if let Ok(FileMetadata::Single { metadata, .. }) = result {
            assert_eq!(
                metadata.materialization,
                Some(crate::config::Materialization::Table)
            );
            assert!(
                metadata.batched.is_none(),
                "malformed batched block must be stripped in recovery"
            );
        }
    }

    /// `materialization: ephemeral` + `timeseries:` is `MalformedTimeseries`.
    #[test]
    fn test_timeseries_on_ephemeral_errors() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Ephemeral),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
            .expect_err("ephemeral + timeseries must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(
            err.to_string().contains("ephemeral"),
            "Error must mention ephemeral"
        );
    }

    /// `partition_column` absent from the model's SQL body is `MalformedTimeseries`.
    #[test]
    fn test_timeseries_partition_column_must_project() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        // SQL body does NOT contain "event_date"
        let err = validate_timeseries(&metadata, "SELECT event_ts, user_id FROM foo")
            .expect_err("partition_column absent from SQL must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(
            err.to_string().contains("event_date"),
            "Error must name the missing column, got: {}",
            err
        );
    }

    // ── keyed frontmatter tests ───────────────────────────────────────────────

    /// `materialization: cumulative_aggregate` is no longer valid — must fail.
    /// The new opt-in is `materialization: table` + `refresh: keyed`.
    #[test]
    fn test_cumulative_aggregate_frontmatter_is_rejected() {
        let source = r#"---
materialization: cumulative_aggregate
---
SELECT device_id, user_id, COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id, user_id"#;

        let result = extract_file_metadata(source);
        assert!(
            result.is_err(),
            "`materialization: cumulative_aggregate` must be rejected (unknown value)"
        );
    }

    /// `materialization: table` + `refresh: incremental` + `grain: key`
    /// parses cleanly (the former `refresh: keyed`).
    #[test]
    fn test_refresh_keyed_frontmatter_parses() {
        let source = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, user_id, COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id, user_id"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(
                    metadata.materialization,
                    Some(crate::config::Materialization::Table)
                );
                assert_eq!(
                    metadata.refresh,
                    Some(crate::config::RefreshStrategy::Incremental)
                );
                assert_eq!(metadata.grain, Some(crate::config::Grain::Key));
                assert!(metadata.timeseries.is_none());
                assert!(metadata.batched.is_none());
            }
            _ => panic!("Expected Single variant"),
        }
    }

    /// A model with `refresh: keyed` + a `timeseries:` block
    /// emits `KeyedForbidsTimeseries`.
    #[test]
    fn test_keyed_forbids_timeseries() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(crate::config::RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Key),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
            .expect_err("refresh: keyed + timeseries must error");
        assert!(
            matches!(err, MetadataError::KeyedForbidsTimeseries),
            "Expected KeyedForbidsTimeseries, got: {}",
            err
        );
    }

    /// A model with `refresh: keyed` + a `batched:` block
    /// emits `KeyedForbidsBatched`.
    #[test]
    fn test_keyed_forbids_batched() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(crate::config::RefreshStrategy::Incremental),
            grain: Some(crate::config::Grain::Key),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT * FROM foo")
            .expect_err("refresh: keyed + batched: must error");
        assert!(
            matches!(err, MetadataError::KeyedForbidsBatched),
            "Expected KeyedForbidsBatched, got: {}",
            err
        );
    }

    /// `materialization: table` + `refresh: materialized_view` parses cleanly.
    #[test]
    fn test_refresh_materialized_view_frontmatter_parses() {
        let source = r#"---
materialization: table
refresh: materialized_view
---
SELECT device_id, user_id, COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id, user_id"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(
                    metadata.materialization,
                    Some(crate::config::Materialization::Table)
                );
                assert_eq!(
                    metadata.refresh,
                    Some(crate::config::RefreshStrategy::MaterializedView)
                );
                assert!(metadata.timeseries.is_none());
                assert!(metadata.batched.is_none());
            }
            _ => panic!("Expected Single variant"),
        }
    }

    /// A model with `refresh: materialized_view` + a `timeseries:` block
    /// emits `MaterializedViewForbidsTimeseries`.
    #[test]
    fn test_materialized_view_forbids_timeseries() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(crate::config::RefreshStrategy::MaterializedView),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
            .expect_err("refresh: materialized_view + timeseries must error");
        assert!(
            matches!(err, MetadataError::MaterializedViewForbidsTimeseries),
            "Expected MaterializedViewForbidsTimeseries, got: {}",
            err
        );
    }

    /// A model with `refresh: materialized_view` + a `batched:` block
    /// emits `MaterializedViewForbidsBatched`.
    #[test]
    fn test_materialized_view_forbids_batched() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            refresh: Some(crate::config::RefreshStrategy::MaterializedView),
            batched: Some(crate::config::BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: crate::config::BatchedSafetyOverrides::default(),
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT * FROM foo")
            .expect_err("refresh: materialized_view + batched: must error");
        assert!(
            matches!(err, MetadataError::MaterializedViewForbidsBatched),
            "Expected MaterializedViewForbidsBatched, got: {}",
            err
        );
    }

    #[test]
    fn test_frontmatter_with_backfill() {
        let source = r#"---
name: test_model
materialization: table
columns:
  full_name:
    backfill: "COALESCE(first_name || ' ' || last_name, '')"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let col = metadata.columns.get("full_name").unwrap();
                assert_eq!(
                    col.backfill.as_ref().unwrap(),
                    "COALESCE(first_name || ' ' || last_name, '')"
                );
            }
            _ => panic!("Expected Single variant"),
        }
    }

    // ── BUG-026: week_start value-domain validation ──────────────────────────

    /// `week_start: wednesday` (invalid domain) on `granularity: week` must
    /// emit `MalformedTimeseries`.
    #[test]
    fn test_week_start_invalid_value_rejected() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Week,
                week_start: Some(crate::config::Weekday::Wednesday),
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let err = validate_timeseries(&metadata, "SELECT dt, ts FROM foo")
            .expect_err("week_start: wednesday must error");
        assert!(
            matches!(err, MetadataError::MalformedTimeseries { .. }),
            "Expected MalformedTimeseries, got: {}",
            err
        );
        assert!(
            err.to_string().contains("week_start"),
            "Error must mention week_start, got: {}",
            err
        );
    }

    /// `week_start: monday` is valid — no error.
    #[test]
    fn test_week_start_monday_accepted() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Week,
                week_start: Some(crate::config::Weekday::Monday),
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        validate_timeseries(&metadata, "SELECT dt, ts FROM foo")
            .expect("week_start: monday must be accepted");
    }

    /// `week_start: sunday` is valid — no error.
    #[test]
    fn test_week_start_sunday_accepted() {
        let metadata = ModelMetadata {
            materialization: Some(crate::config::Materialization::Table),
            timeseries: Some(crate::config::TimeseriesConfig {
                event_time_column: "ts".to_string(),
                partition_column: "dt".to_string(),
                granularity: crate::config::Granularity::Week,
                week_start: Some(crate::config::Weekday::Sunday),
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        validate_timeseries(&metadata, "SELECT dt, ts FROM foo")
            .expect("week_start: sunday must be accepted");
    }
}
