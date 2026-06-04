//! Sidecar YAML parsing for smelt seed CSV files.
//!
//! A sidecar YAML (`<name>.yml`) next to a `<name>.csv` file declares:
//! - `columns:` — optional explicit column schema (names, types, nullability)
//! - `materialization:` — `table` (default), `ephemeral`, `view`, or `materialized_view`
//!   (the latter two are parsed but rejected at load-time validation)
//! - `name:` — **forbidden** — emits a hard parse error if present
//!
//! References: seeds.md §"Sidecar YAML — seed-specific keys"

use serde::Deserialize;
use smelt_types::{parse_type, DataType};
use std::path::{Path, PathBuf};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Materialization setting for a seed as declared in its sidecar YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedMaterialization {
    /// Default: load the CSV into a database table on `smelt seed`.
    Table,
    /// Inline: at compile time, references to this seed are rewritten to a
    /// `VALUES (…)` CTE. No table is ever created.
    Ephemeral,
    /// Parsed but rejected at validation time (seeds.md §"Materialization for seeds").
    View,
    /// Parsed but rejected at validation time (seeds.md §"Materialization for seeds").
    MaterializedView,
}

/// A single column declaration from a seed sidecar YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarColumn {
    pub name: String,
    pub data_type: DataType,
    /// Whether NULL values are permitted at load time (default: `true`).
    pub nullable: bool,
    pub description: Option<String>,
}

/// Parsed content of a seed sidecar YAML file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSidecar {
    pub materialization: SeedMaterialization,
    /// `None` = full type inference from the CSV.
    /// `Some(cols)` = pinned schema; column set must match CSV header exactly.
    pub columns: Option<Vec<SidecarColumn>>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while reading/parsing a seed sidecar YAML.
#[derive(Debug, Error)]
pub enum SidecarError {
    #[error(
        "Seed sidecars do not support `name:` — seed DB names are derived from the workspace path"
    )]
    NameKeyForbidden,

    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML parse error in {path}: {message}")]
    YamlParse { path: PathBuf, message: String },

    #[error("Unknown type '{type_str}' in sidecar column '{column}'")]
    UnknownType { type_str: String, column: String },

    #[error(
        "Unknown materialization '{value}' — valid values are: table, ephemeral, view, materialized_view"
    )]
    UnknownMaterialization { value: String },
}

// ---------------------------------------------------------------------------
// Internal deserialization helpers
// ---------------------------------------------------------------------------

/// Raw YAML struct — catches `name:` so we can error explicitly.
#[derive(Deserialize)]
struct RawSidecar {
    /// Presence of this key is a hard error for seeds.
    name: Option<serde_yaml::Value>,
    #[serde(default)]
    materialization: Option<String>,
    #[serde(default)]
    columns: Option<Vec<RawColumn>>,
    // Absorb description silently (valid key, no effect on loading logic).
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
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

/// Parse a sidecar YAML file from disk.
///
/// Returns `Err(SidecarError::NameKeyForbidden)` if the YAML contains a `name:` key.
/// Returns `Err(SidecarError::UnknownType { … })` if a column type string is not
/// recognised by smelt's type vocabulary.
pub fn parse_sidecar(path: &Path) -> Result<SeedSidecar, SidecarError> {
    let text = std::fs::read_to_string(path).map_err(|e| SidecarError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse_sidecar_from_str(&text, path)
}

/// Parse sidecar YAML from an in-memory string (for tests and LSP).
pub fn parse_sidecar_from_str(text: &str, path: &Path) -> Result<SeedSidecar, SidecarError> {
    let raw: RawSidecar = serde_yaml::from_str(text).map_err(|e| SidecarError::YamlParse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    // `name:` is forbidden on seed sidecars.
    if raw.name.is_some() {
        return Err(SidecarError::NameKeyForbidden);
    }

    let materialization = match raw.materialization.as_deref().unwrap_or("table") {
        "table" => SeedMaterialization::Table,
        "ephemeral" => SeedMaterialization::Ephemeral,
        "view" => SeedMaterialization::View,
        "materialized_view" => SeedMaterialization::MaterializedView,
        other => {
            return Err(SidecarError::UnknownMaterialization {
                value: other.to_string(),
            });
        }
    };

    let columns = raw
        .columns
        .map(|cols| {
            cols.into_iter()
                .map(|c| {
                    let data_type = match &c.type_str {
                        None => DataType::Text, // default to VARCHAR when no type given
                        Some(ts) => parse_type(ts).map_err(|_| SidecarError::UnknownType {
                            type_str: ts.clone(),
                            column: c.name.clone(),
                        })?,
                    };
                    Ok(SidecarColumn {
                        name: c.name,
                        data_type,
                        nullable: c.nullable,
                        description: c.description,
                    })
                })
                .collect::<Result<Vec<_>, SidecarError>>()
        })
        .transpose()?;

    Ok(SeedSidecar {
        materialization,
        columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn name_key_is_forbidden() {
        let text = "name: foo.bar\nmaterialization: table\n";
        let err = parse_sidecar_from_str(text, Path::new("test.yml")).unwrap_err();
        assert!(
            matches!(err, SidecarError::NameKeyForbidden),
            "Expected NameKeyForbidden, got {err:?}"
        );
    }

    #[test]
    fn default_materialization_is_table() {
        let text = "columns:\n  - name: x\n    type: INTEGER\n";
        let sidecar = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap();
        assert_eq!(sidecar.materialization, SeedMaterialization::Table);
    }

    #[test]
    fn ephemeral_materialization() {
        let text = "materialization: ephemeral\n";
        let sidecar = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap();
        assert_eq!(sidecar.materialization, SeedMaterialization::Ephemeral);
    }

    #[test]
    fn view_materialization_parses() {
        let text = "materialization: view\n";
        let sidecar = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap();
        assert_eq!(sidecar.materialization, SeedMaterialization::View);
    }

    #[test]
    fn materialized_view_materialization_parses() {
        let text = "materialization: materialized_view\n";
        let sidecar = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap();
        assert_eq!(
            sidecar.materialization,
            SeedMaterialization::MaterializedView
        );
    }

    #[test]
    fn column_nullable_defaults_to_true() {
        let text = "columns:\n  - name: x\n    type: INTEGER\n";
        let sidecar = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap();
        let cols = sidecar.columns.unwrap();
        assert!(cols[0].nullable, "nullable should default to true");
    }

    #[test]
    fn unknown_type_returns_error() {
        let text = "columns:\n  - name: x\n    type: NOTATYPE\n";
        let err = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap_err();
        assert!(
            matches!(err, SidecarError::UnknownType { .. }),
            "Expected UnknownType, got {err:?}"
        );
    }

    /// BUG-030: an unknown `materialization:` value must be a hard error, not
    /// silently coerced to `table`.  Previously the `other =>` arm silently
    /// fell back to `SeedMaterialization::Table`; now it must return
    /// `SidecarError::UnknownMaterialization`.
    #[test]
    fn unknown_materialization_is_error_not_silent_table() {
        let text = "materialization: bogus\n";
        let err = parse_sidecar_from_str(text, Path::new("x.yml")).unwrap_err();
        assert!(
            matches!(
                err,
                SidecarError::UnknownMaterialization { ref value } if value == "bogus"
            ),
            "Expected UnknownMaterialization {{ value: \"bogus\" }}, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("bogus"),
            "error message should include the unknown value, got: {msg}"
        );
    }
}
