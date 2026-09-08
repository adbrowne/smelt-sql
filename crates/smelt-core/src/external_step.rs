//! Externally-produced sources (black-box steps).
//!
//! Per-entity discovery and validation for the `external_step:` declaration.
//! Reference: `docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)".

use crate::discovery::ModelDiscovery;
use crate::sources::SourceInfo;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A step's cadence — how often its producer intends to run.
///
/// Reuses [`crate::config::DataLatency`]'s interval grammar (`'1 day'`), but
/// is stored as a distinct newtype so the spec's cadence-vs-lateness
/// distinction (`sources.md`'s `cadence` row) cannot collapse by type
/// aliasing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepCadence {
    /// Cadence in seconds.
    pub seconds: u64,
    /// Original string representation (for display).
    pub display: String,
}

/// A discovered external-step declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStepInfo {
    /// Absolute path to the `.yml` file on disk.
    pub path: PathBuf,
    /// Address segments stripped from scan-root to stem.
    pub address_segments: Vec<String>,
    /// Optional free-text description.
    pub description: Option<String>,
    /// Source addresses this step produces, as written (`smelt.<path>` form).
    pub produces: Vec<String>,
    /// Argv the step is invoked with. Opaque to smelt — never parsed or
    /// type-checked.
    pub command: Vec<String>,
    /// How often the producer intends to run. Absent = unknown.
    pub cadence: Option<StepCadence>,
}

/// Errors from parsing or validating an external-step declaration.
#[derive(Debug, Error)]
pub enum ExternalStepError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML parse error in {path}: {message}")]
    YamlParse { path: PathBuf, message: String },

    #[error("`external_step.produces:` must be a non-empty list of source addresses")]
    EmptyProduces,

    #[error("`external_step.command:` must be a non-empty argv list")]
    EmptyCommand,

    #[error("`external_step.command:` must be a list of strings, not a scalar")]
    CommandNotList,

    #[error("`columns:` is not allowed alongside `external_step:` on the same file")]
    ColumnsAlongsideExternalStep,

    #[error("invalid `external_step.cadence` interval: '{0}'")]
    UnparseableCadence(String),

    #[error("`external_step.produces:` entry '{0}' does not resolve to a declared source")]
    ProducesUnknownSource(String),

    #[error(
        "source '{source_address}' is produced by more than one external step: {step_a} and {step_b}"
    )]
    ProducerConflict {
        source_address: String,
        step_a: PathBuf,
        step_b: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// Internal deserialization helpers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExternalStepFile {
    external_step: RawExternalStep,
    /// Presence of this key alongside `external_step:` is a hard error — a
    /// step file declares no output shape.
    #[serde(default)]
    columns: Option<serde_yaml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExternalStep {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    produces: Option<Vec<String>>,
    #[serde(default)]
    command: Option<serde_yaml::Value>,
    #[serde(default)]
    cadence: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a single external-step YAML file from disk.
///
/// The returned `ExternalStepInfo` has `address_segments` computed from the
/// file stem only (no scan-root stripping). Callers that need the full
/// address should use [`discover_external_steps`] instead.
pub fn parse_external_step_yaml(path: &Path) -> Result<ExternalStepInfo, ExternalStepError> {
    let text = std::fs::read_to_string(path).map_err(|e| ExternalStepError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let raw: RawExternalStepFile =
        serde_yaml::from_str(&text).map_err(|e| ExternalStepError::YamlParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    if raw.columns.is_some() {
        return Err(ExternalStepError::ColumnsAlongsideExternalStep);
    }

    let produces = raw.external_step.produces.unwrap_or_default();
    if produces.is_empty() {
        return Err(ExternalStepError::EmptyProduces);
    }

    let command = match raw.external_step.command {
        None => return Err(ExternalStepError::EmptyCommand),
        Some(serde_yaml::Value::Sequence(seq)) => {
            let mut argv = Vec::with_capacity(seq.len());
            for item in seq {
                match item {
                    serde_yaml::Value::String(s) => argv.push(s),
                    _ => return Err(ExternalStepError::CommandNotList),
                }
            }
            argv
        }
        Some(_) => return Err(ExternalStepError::CommandNotList),
    };
    if command.is_empty() {
        return Err(ExternalStepError::EmptyCommand);
    }

    let cadence = match raw.external_step.cadence {
        None => None,
        Some(s) => Some(
            crate::config::DataLatency::parse(&s)
                .map(|dl| StepCadence {
                    seconds: dl.seconds,
                    display: dl.display,
                })
                .ok_or_else(|| ExternalStepError::UnparseableCadence(s.clone()))?,
        ),
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    Ok(ExternalStepInfo {
        path: path.to_path_buf(),
        address_segments: vec![stem],
        description: raw.external_step.description,
        produces,
        command,
        cadence,
    })
}

/// Walk the project root and return every candidate external-step `.yml`
/// file — a standalone YAML carrying a top-level `external_step:` block.
fn candidate_external_step_yaml_files(project_dir: &Path) -> Vec<PathBuf> {
    use crate::discovery::project_root_files_by_dir;
    use crate::resolver::{classify, EntityKind};

    let mut candidates = Vec::new();
    for (_, files) in project_root_files_by_dir(project_dir) {
        for file_path in &files {
            if matches!(
                classify(file_path, None, &files),
                Some(EntityKind::ExternalStep)
            ) {
                candidates.push(file_path.clone());
            }
        }
    }
    candidates
}

/// Discover and parse every external-step declaration under `project_dir`.
/// Files that fail to parse are silently skipped (surfaced via
/// [`discover_external_step_errors`]).
pub fn discover_external_steps(project_dir: &Path, paths: &[String]) -> Vec<ExternalStepInfo> {
    let mut steps = Vec::new();

    for file_path in candidate_external_step_yaml_files(project_dir) {
        let mut info = match parse_external_step_yaml(&file_path) {
            Ok(i) => i,
            Err(_) => continue,
        };
        info.address_segments =
            ModelDiscovery::compute_address_segments(&file_path, project_dir, paths);
        steps.push(info);
    }

    steps.sort_by(|a, b| a.address_segments.cmp(&b.address_segments));
    steps
}

/// Discover every external-step candidate file that **fails** to parse,
/// paired with its [`ExternalStepError`]. Sorted by path for deterministic
/// diagnostic ordering.
///
/// Unlike `discover_source_errors`, candidate discovery here needs no
/// scan-root list — `candidate_external_step_yaml_files` classifies by
/// content, not by address — so this takes only `project_dir`.
pub fn discover_external_step_errors(project_dir: &Path) -> Vec<(PathBuf, ExternalStepError)> {
    let mut errors: Vec<(PathBuf, ExternalStepError)> = Vec::new();

    for file_path in candidate_external_step_yaml_files(project_dir) {
        if let Err(e) = parse_external_step_yaml(&file_path) {
            errors.push((file_path, e));
        }
    }

    errors.sort_by(|a, b| a.0.cmp(&b.0));
    errors
}

/// Cross-entity validation over the whole project's step and source sets
/// (`sources.md` §"Externally-produced sources"): every `produces:` entry
/// must resolve to a declared source, and a source may be named by at most
/// one step. Pure function — called as a second pass from the Salsa query,
/// mirroring how `project_source_diagnostics` already runs the per-target
/// `name:`-key check.
///
/// Results are ordered by the step's own path, then by declaration order
/// within that step's `produces:` list — a `ProducerConflict` is anchored at
/// the later-sorted (second-seen) step.
pub fn validate_external_steps(
    steps: &[ExternalStepInfo],
    sources: &[SourceInfo],
) -> Vec<(PathBuf, ExternalStepError)> {
    let source_addresses: std::collections::BTreeSet<String> = sources
        .iter()
        .map(|s| s.address_segments.join("."))
        .collect();

    let mut sorted_steps: Vec<&ExternalStepInfo> = steps.iter().collect();
    sorted_steps.sort_by(|a, b| a.path.cmp(&b.path));

    let mut errors = Vec::new();
    let mut producer_of: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();

    for step in sorted_steps {
        for produces in &step.produces {
            let resolved = produces
                .strip_prefix("smelt.")
                .filter(|a| source_addresses.contains(*a));
            let Some(addr) = resolved else {
                errors.push((
                    step.path.clone(),
                    ExternalStepError::ProducesUnknownSource(produces.clone()),
                ));
                continue;
            };
            let addr = addr.to_string();
            if let Some(existing) = producer_of.get(&addr) {
                errors.push((
                    step.path.clone(),
                    ExternalStepError::ProducerConflict {
                        source_address: produces.clone(),
                        step_a: existing.clone(),
                        step_b: step.path.clone(),
                    },
                ));
            } else {
                producer_of.insert(addr, step.path.clone());
            }
        }
    }

    errors
}
