//! Per-entity source YAML loading for smelt.
//!
//! Phase 6: sources are now discovered from the filesystem as standalone `.yml`
//! files (no sibling `.csv`). The old aggregate `sources.yml` at the project
//! root is no longer supported — its presence is a hard error.
//!
//! References: docs/specs/sources.md §"Source YAML shape" and §"Filesystem layout"

use crate::config::{DataLatency, TimeseriesConfig};
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

impl SourceNameOverride {
    /// Validate that every key in a `PerTarget` map names a declared target.
    ///
    /// Returns `Some(SourceError::InvalidTargetName(key))` for the first key
    /// that is absent from `declared_targets`, or `None` if all keys are valid.
    /// `Literal` variants always return `None` (no target-name keys to check).
    pub fn validate_target_keys(&self, declared_targets: &[&str]) -> Option<SourceError> {
        if let SourceNameOverride::PerTarget(map) = self {
            for key in map.keys() {
                if !declared_targets.contains(&key.as_str()) {
                    return Some(SourceError::InvalidTargetName(key.clone()));
                }
            }
        }
        None
    }
}

/// A source's declared mutation profile kind — the one non-derivable
/// world-fact on the input-consumption axis (`docs/specs/models.md`
/// §"Input-consumption axis"; `docs/specs/model_properties.md` §"Catalogued
/// inputs"). Undeclared (`SourceInfo.mutation_profile == None`) is the
/// conservative default: `smelt-logical`'s input-delta discovery (F9) treats
/// an unclocked source with no declared profile as `mutable` and falls back
/// to a whole-relation snapshot-diff rather than an optimistic delta that
/// could silently drop rows.
///
/// The YAML wire name for `Mutable` is `mutable_snapshot` (`sources.md`
/// §"`mutation_profile` — the structured block": the rename says *what a
/// read observes* — a snapshot — which is exactly the fact that refuses
/// folding). The Rust variant name stays `Mutable` so existing
/// `smelt-logical` pattern matches on this type by Rust name are unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationProfile {
    /// Rows are only ever appended, never updated or deleted in place.
    AppendOnly,
    /// Rows may be updated or deleted in place — only a full re-scan sees
    /// every change. Wire name: `mutable_snapshot`.
    #[serde(rename = "mutable_snapshot")]
    Mutable,
    /// The source itself exposes a change-data feed (CDC/CDF): a run can read
    /// only the rows that changed since the last run.
    ChangeFeed,
}

impl MutationProfile {
    /// The wire/YAML spelling of this kind, used in error messages.
    fn wire_name(self) -> &'static str {
        match self {
            MutationProfile::AppendOnly => "append_only",
            MutationProfile::Mutable => "mutable_snapshot",
            MutationProfile::ChangeFeed => "change_feed",
        }
    }
}

/// Whether a delivered row of an `append_only` source may be redelivered.
/// Sub-fact of [`SourceMutationProfile`] (`sources.md` §"`mutation_profile` —
/// the structured block"). Default: `AtLeastOnce` (the conservative
/// posture — a lazy declaration never licenses a cheaper technique than its
/// most conservative value would, per `sources.md` Constraint 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Redelivery {
    /// Each row arrives exactly once.
    None,
    /// A delivered row may arrive again (the conservative default).
    #[default]
    AtLeastOnce,
}

/// The delivery-contract recurrence bound: every pair of rows sharing `key`
/// lies within `window` of each other on the event-time axis. Valid under
/// any `mutation_profile.kind` (`sources.md` §"`mutation_profile` — the
/// structured block"). Consumed by key temporal locality
/// (`incremental_models.md`); always runtime-checked, never trusted
/// (`KeyedRecurrenceBoundViolated`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecurrence {
    /// Key column(s) the recurrence bound applies to (composite-valued).
    pub key: Vec<String>,
    /// The recurrence window on the event-time axis.
    pub window: DataLatency,
}

/// The structured `mutation_profile` block (`sources.md` §"`mutation_profile`
/// — the structured block"): a `kind` plus the sub-facts admission consumes.
/// Bare-string shorthand (`mutation_profile: append_only`) normalizes to
/// `SourceMutationProfile { kind, ..conservative defaults }` — there is no
/// separate internal representation for the shorthand form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMutationProfile {
    /// `append_only | mutable_snapshot | change_feed`.
    pub kind: MutationProfile,
    /// `append_only` sub-fact: how far behind the clock a row can arrive.
    /// Also populated from the top-level `source_lateness:` alias
    /// (`sources.md`'s `source_lateness` row) when the block itself does
    /// not declare `lateness:`.
    pub lateness: Option<DataLatency>,
    /// `append_only` sub-fact: whether a delivered row may be redelivered.
    /// Default: [`Redelivery::AtLeastOnce`].
    pub redelivery: Redelivery,
    /// `change_feed` sub-fact: does the feed carry deletes/updates as
    /// retraction events? Default: `true` (the conservative posture).
    pub retractions: bool,
    /// `change_feed` sub-fact: is the feed ordered by its offset column?
    pub ordered: Option<bool>,
    /// `change_feed` sub-fact: stable per-delta identity column(s) — the
    /// dedup key of the ledger's never-fold-twice obligation.
    pub delta_identity: Option<Vec<String>>,
    /// Delivery-contract recurrence bound, valid under any `kind`.
    pub key_recurrence: Option<KeyRecurrence>,
}

impl SourceMutationProfile {
    /// The bare-string-shorthand normalization: `{ kind, ..conservative
    /// defaults }`.
    pub fn from_kind(kind: MutationProfile) -> Self {
        SourceMutationProfile {
            kind,
            lateness: None,
            redelivery: Redelivery::default(),
            retractions: true,
            ordered: None,
            delta_identity: None,
            key_recurrence: None,
        }
    }
}

/// Where a source's pipeline publishes a completeness marker
/// (`sources.md`'s `watermark:` row).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watermark {
    /// `<schema.table.column>` or bare `column` naming the completeness
    /// marker.
    pub complete_through: String,
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
    /// Declared mutation profile: kind (append-only / mutable-snapshot /
    /// change-feed) plus its structured sub-facts. See
    /// [`SourceMutationProfile`]. `None` is the undeclared/unknown case — the
    /// fail-closed default consumed by `smelt-logical`'s input-delta
    /// discovery (F9).
    pub mutation_profile: Option<SourceMutationProfile>,
    /// Declared source-lateness margin — the term of the reach split
    /// (`docs/specs/model_properties.md` §"Unified bound/reach derivation").
    /// Reuses [`DataLatency`]'s existing fail-loud interval parser; `None`
    /// (absent) means no declared lateness margin (default 0). This is the
    /// raw top-level `source_lateness:` key as written; when a
    /// `mutation_profile:` block is also declared, this value is folded into
    /// [`SourceMutationProfile::lateness`] as the alias (`sources.md`'s
    /// `source_lateness` row) — declaring both is a `MalformedSource` error.
    pub source_lateness: Option<DataLatency>,
    /// Where the source's pipeline publishes a completeness marker
    /// (`sources.md`'s `watermark:` row). Absent = derived watermark
    /// (`max(partition_column)` processed so far).
    pub watermark: Option<Watermark>,
    /// Row identity of the source, composite-valued (a single declared
    /// string normalizes to a one-element list). Licenses 1:1
    /// join-cardinality proofs and dedup-free key-addressed merges.
    /// Verified, never trusted (`sources.md` §Semantics "The trust rule").
    pub unique_key: Option<Vec<String>>,
    /// How far back the source can be re-read. Absent = assumed fully
    /// replayable (`sources.md`'s trusted-replayable default).
    pub retention: Option<DataLatency>,
}

impl SourceInfo {
    /// Returns the fully-qualified database name for this source given the active
    /// target name and schema.
    ///
    /// Resolution rules (spec: `docs/specs/sources.md` §"Target-aware `name:` override"):
    /// - `Literal(s)` — returns `s` verbatim regardless of `target_name`.
    /// - `PerTarget(map)` — looks up `target_name`; if found returns that value;
    ///   if absent falls back to the default mapping
    ///   `<target_schema>.<address_segments.join("_")>`.
    /// - `None` — default mapping.
    pub fn db_name_for_target(&self, target_name: &str, target_schema: &str) -> String {
        match &self.name_override {
            Some(SourceNameOverride::Literal(s)) => s.clone(),
            Some(SourceNameOverride::PerTarget(map)) => {
                if let Some(v) = map.get(target_name) {
                    v.clone()
                } else {
                    format!("{}.{}", target_schema, self.address_segments.join("_"))
                }
            }
            None => format!("{}.{}", target_schema, self.address_segments.join("_")),
        }
    }

    /// Returns the fully-qualified database name for this source.
    ///
    /// This is a shim for callers that do not yet pass the active target name.
    /// For `Literal` overrides it returns the literal verbatim; for all other
    /// cases it returns the default mapping `<target_schema>.<segs.join("_")>`.
    ///
    /// Prefer `db_name_for_target` when the active target name is available.
    pub fn db_name(&self, target_schema: &str) -> String {
        self.db_name_for_target("", target_schema)
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

    #[error("`name:` map key '{0}' names no declared target in smelt.yml")]
    InvalidTargetName(String),

    #[error(
        "both `source_lateness:` and `mutation_profile.lateness` are declared — declare lateness once (`source_lateness:` is an alias for `mutation_profile.lateness`)"
    )]
    LatenessDoubleDeclared,

    #[error(
        "`mutation_profile.{field}` is only valid for `kind: {expected_kind}`, but this source declares `kind: {actual_kind}`"
    )]
    MutationProfileSubFactWrongKind {
        field: &'static str,
        expected_kind: &'static str,
        actual_kind: &'static str,
    },
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

    /// Declared mutation profile — bare-string shorthand or the structured
    /// block. See [`RawMutationProfile`].
    #[serde(default)]
    mutation_profile: Option<RawMutationProfile>,

    /// Declared source-lateness margin — see [`DataLatency`]. Alias for
    /// `mutation_profile.lateness`.
    #[serde(default)]
    source_lateness: Option<DataLatency>,

    /// Where the source's pipeline publishes a completeness marker.
    #[serde(default)]
    watermark: Option<RawWatermark>,

    /// Row identity of the source — single string or list, both accepted.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    unique_key: Option<Vec<String>>,

    /// How far back the source can be re-read.
    #[serde(default)]
    retention: Option<DataLatency>,
}

/// `mutation_profile:` accepts either the bare-string shorthand
/// (`mutation_profile: append_only`) or the structured block. Both forms
/// normalize into one [`SourceMutationProfile`] in [`parse_source_yaml`] —
/// there is no dual internal representation downstream of that
/// normalization.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawMutationProfile {
    /// `mutation_profile: append_only` (or `mutable_snapshot` / `change_feed`).
    Shorthand(MutationProfile),
    /// The structured block with sub-facts.
    Full(RawMutationProfileBlock),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMutationProfileBlock {
    kind: MutationProfile,
    #[serde(default)]
    lateness: Option<DataLatency>,
    #[serde(default)]
    redelivery: Option<Redelivery>,
    #[serde(default)]
    retractions: Option<bool>,
    #[serde(default)]
    ordered: Option<bool>,
    #[serde(default)]
    delta_identity: Option<Vec<String>>,
    #[serde(default)]
    key_recurrence: Option<RawKeyRecurrence>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyRecurrence {
    #[serde(deserialize_with = "string_or_vec")]
    key: Vec<String>,
    window: DataLatency,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWatermark {
    complete_through: String,
}

/// A single string is sugar for a one-element list — the composite-valued
/// convention shared by `unique_key:` and `mutation_profile.key_recurrence.key`
/// (`sources.md` §"Design" "Composite `unique_key` from day one").
#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    Single(String),
    Multi(Vec<String>),
}

impl From<StringOrVec> for Vec<String> {
    fn from(v: StringOrVec) -> Self {
        match v {
            StringOrVec::Single(s) => vec![s],
            StringOrVec::Multi(v) => v,
        }
    }
}

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    StringOrVec::deserialize(deserializer).map(Into::into)
}

fn opt_string_or_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<StringOrVec>::deserialize(deserializer).map(|opt| opt.map(Into::into))
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

/// Normalize a raw `mutation_profile:` value (shorthand or structured block)
/// plus the top-level `source_lateness:` alias into one [`SourceMutationProfile`].
///
/// Validates the trust-rule shape (`sources.md` §"Diagnostic codes"
/// `MalformedSource` row): a sub-fact declared for the wrong `kind`, or both
/// `source_lateness:` and `mutation_profile.lateness` declared.
fn normalize_mutation_profile(
    raw_profile: Option<RawMutationProfile>,
    raw_source_lateness: &Option<DataLatency>,
) -> Result<Option<SourceMutationProfile>, SourceError> {
    let Some(raw_profile) = raw_profile else {
        return Ok(None);
    };

    let mut profile = match raw_profile {
        RawMutationProfile::Shorthand(kind) => SourceMutationProfile::from_kind(kind),
        RawMutationProfile::Full(block) => {
            let kind = block.kind;

            if kind != MutationProfile::AppendOnly {
                if block.lateness.is_some() {
                    return Err(SourceError::MutationProfileSubFactWrongKind {
                        field: "lateness",
                        expected_kind: "append_only",
                        actual_kind: kind.wire_name(),
                    });
                }
                if block.redelivery.is_some() {
                    return Err(SourceError::MutationProfileSubFactWrongKind {
                        field: "redelivery",
                        expected_kind: "append_only",
                        actual_kind: kind.wire_name(),
                    });
                }
            }

            if kind != MutationProfile::ChangeFeed {
                if block.retractions.is_some() {
                    return Err(SourceError::MutationProfileSubFactWrongKind {
                        field: "retractions",
                        expected_kind: "change_feed",
                        actual_kind: kind.wire_name(),
                    });
                }
                if block.ordered.is_some() {
                    return Err(SourceError::MutationProfileSubFactWrongKind {
                        field: "ordered",
                        expected_kind: "change_feed",
                        actual_kind: kind.wire_name(),
                    });
                }
                if block.delta_identity.is_some() {
                    return Err(SourceError::MutationProfileSubFactWrongKind {
                        field: "delta_identity",
                        expected_kind: "change_feed",
                        actual_kind: kind.wire_name(),
                    });
                }
            }

            SourceMutationProfile {
                kind,
                lateness: block.lateness,
                redelivery: block.redelivery.unwrap_or_default(),
                retractions: block.retractions.unwrap_or(true),
                ordered: block.ordered,
                delta_identity: block.delta_identity,
                key_recurrence: block.key_recurrence.map(|kr| KeyRecurrence {
                    key: kr.key,
                    window: kr.window,
                }),
            }
        }
    };

    if profile.lateness.is_some() && raw_source_lateness.is_some() {
        return Err(SourceError::LatenessDoubleDeclared);
    }
    if profile.lateness.is_none() {
        profile.lateness = raw_source_lateness.clone();
    }

    Ok(Some(profile))
}

fn default_nullable() -> bool {
    // Source columns default to NOT NULL when nullable is not declared.
    // External source columns are typically non-null for structured data;
    // the conservative nullable=true default propagated through downstream
    // CASTs and produced false-positive D-52 partition-column diagnostics.
    false
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

    let mutation_profile = normalize_mutation_profile(raw.mutation_profile, &raw.source_lateness)?;

    Ok(SourceInfo {
        path: path.to_path_buf(),
        address_segments: vec![stem],
        columns,
        description: raw.description,
        name_override: raw.name,
        tags: raw.tags,
        timeseries: raw.timeseries,
        mutation_profile,
        source_lateness: raw.source_lateness,
        watermark: raw.watermark.map(|w| Watermark {
            complete_through: w.complete_through,
        }),
        unique_key: raw.unique_key,
        retention: raw.retention,
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
