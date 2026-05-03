//! Validation of CSV data against a seed sidecar YAML.
//!
//! Implements seeds.md Semantics §1–§3:
//! 1. Column set must match CSV header exactly when pinned.
//! 2. Type-coercion failures are hard errors with file/row/column pointer.
//! 3. `nullable: false` is a load-time check.
//! 4. `view` / `materialized_view` materializaion is rejected.

use super::csv::CsvRecord;
use super::infer::looks_like_date;
use super::sidecar::{SeedMaterialization, SeedSidecar};
use smelt_types::DataType;
use std::collections::HashSet;
use std::path::Path;
use thiserror::Error;

/// Errors produced by sidecar validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(
        "Column set mismatch: sidecar declares {sidecar_cols:?} but CSV header is {csv_cols:?}"
    )]
    ColumnSetMismatch {
        sidecar_cols: Vec<String>,
        csv_cols: Vec<String>,
    },

    #[error(
        "Type coercion failed in {}, row {row}, column '{column}': \
        value {value:?} cannot be parsed as {expected_type}",
        file.display()
    )]
    CoercionFailed {
        file: std::path::PathBuf,
        row: usize,
        column: String,
        value: String,
        expected_type: String,
    },

    #[error(
        "NULL in non-nullable column '{column}' in {}, row {row}",
        file.display()
    )]
    NullInNonNullableColumn {
        file: std::path::PathBuf,
        row: usize,
        column: String,
    },

    #[error("Materialization '{0}' is not supported for seeds; use 'table' or 'ephemeral'")]
    UnsupportedMaterialization(String),
}

/// Validate CSV data against a seed sidecar.
///
/// Checks, in order:
/// 1. `view`/`materialized_view` materialization → error.
/// 2. If `columns:` is declared, column set must match CSV headers exactly.
/// 3. Each row: for declared columns, check nullable and type coercion.
///
/// `path` is the CSV file path used in error messages.
/// `headers` are the CSV header names.
/// `rows` are the CSV data rows.
/// `sidecar` is the parsed sidecar YAML.
pub fn validate_against_sidecar(
    path: &Path,
    headers: &[String],
    rows: &[CsvRecord],
    sidecar: &SeedSidecar,
) -> Result<(), ValidationError> {
    // §1 — Unsupported materializations
    match &sidecar.materialization {
        SeedMaterialization::View => {
            return Err(ValidationError::UnsupportedMaterialization(
                "view".to_string(),
            ));
        }
        SeedMaterialization::MaterializedView => {
            return Err(ValidationError::UnsupportedMaterialization(
                "materialized_view".to_string(),
            ));
        }
        SeedMaterialization::Table | SeedMaterialization::Ephemeral => {}
    }

    let Some(sidecar_cols) = &sidecar.columns else {
        // No columns declared — full inference, nothing to validate.
        return Ok(());
    };

    // §2 — Column set must match CSV header exactly (same names, any order).
    let sidecar_names: HashSet<&str> = sidecar_cols.iter().map(|c| c.name.as_str()).collect();
    let csv_names: HashSet<&str> = headers.iter().map(|h| h.as_str()).collect();

    if sidecar_names != csv_names {
        return Err(ValidationError::ColumnSetMismatch {
            sidecar_cols: sidecar_cols.iter().map(|c| c.name.clone()).collect(),
            csv_cols: headers.to_vec(),
        });
    }

    // Build a map: header_name → (col_index, SidecarColumn) for per-row checks.
    let col_lookup: Vec<(usize, &super::sidecar::SidecarColumn)> = sidecar_cols
        .iter()
        .map(|sc| {
            let idx = headers
                .iter()
                .position(|h| h == &sc.name)
                .expect("name is in csv_names — position must exist");
            (idx, sc)
        })
        .collect();

    // §3 — Per-row validation.
    for row in rows {
        for (col_idx, sc) in &col_lookup {
            let raw_value = row.fields.get(*col_idx).and_then(|v| v.as_deref());

            match raw_value {
                None => {
                    // NULL cell.
                    if !sc.nullable {
                        return Err(ValidationError::NullInNonNullableColumn {
                            file: path.to_path_buf(),
                            row: row.row_index,
                            column: sc.name.clone(),
                        });
                    }
                    // Nullable → NULL is fine; nothing to coerce.
                }
                Some(v) => {
                    // Non-null: check that the value is parseable as the declared type.
                    if let Err(e) = coerce_check(v, &sc.data_type) {
                        return Err(ValidationError::CoercionFailed {
                            file: path.to_path_buf(),
                            row: row.row_index,
                            column: sc.name.clone(),
                            value: e.value,
                            expected_type: format!("{:?}", sc.data_type),
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

/// A coercion check failure carrying the bad value.
struct CoercionFailure {
    value: String,
}

/// Check whether `value` is parseable as `dt`.
/// Returns `Ok(())` if it can be coerced, `Err(CoercionFailure)` otherwise.
fn coerce_check(value: &str, dt: &DataType) -> Result<(), CoercionFailure> {
    let v = value.trim();
    match dt {
        DataType::Boolean => {
            if matches!(v.to_lowercase().as_str(), "true" | "false") {
                Ok(())
            } else {
                Err(CoercionFailure {
                    value: value.to_string(),
                })
            }
        }
        DataType::Integer => v.parse::<i64>().map(|_| ()).map_err(|_| CoercionFailure {
            value: value.to_string(),
        }),
        DataType::Decimal { .. } => {
            // Accept any fixed-decimal literal (digits, optional sign, optional one dot).
            if v.contains('e') || v.contains('E') {
                return Err(CoercionFailure {
                    value: value.to_string(),
                });
            }
            let s = v.trim_start_matches('-');
            let dots = s.chars().filter(|&c| c == '.').count();
            let digits_ok = s.replace('.', "").chars().all(|c| c.is_ascii_digit());
            if dots <= 1 && digits_ok && !s.is_empty() {
                Ok(())
            } else {
                Err(CoercionFailure {
                    value: value.to_string(),
                })
            }
        }
        DataType::Double => v.parse::<f64>().map(|_| ()).map_err(|_| CoercionFailure {
            value: value.to_string(),
        }),
        DataType::Date => {
            if looks_like_date(v) {
                Ok(())
            } else {
                Err(CoercionFailure {
                    value: value.to_string(),
                })
            }
        }
        DataType::Timestamp { .. } => {
            // Accept `YYYY-MM-DD HH:MM:SS` with optional fractional seconds.
            use chrono::NaiveDateTime;
            let ok = NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S").is_ok()
                || NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S%.f").is_ok();
            if ok {
                Ok(())
            } else {
                Err(CoercionFailure {
                    value: value.to_string(),
                })
            }
        }
        // Text/Varchar — anything goes
        DataType::Text | DataType::Varchar { .. } => Ok(()),
        // Unknown types default to OK (don't block loading)
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seeds::csv::CsvRecord;
    use crate::seeds::sidecar::{SeedMaterialization, SeedSidecar, SidecarColumn};

    fn make_record(row_index: usize, fields: &[Option<&str>]) -> CsvRecord {
        CsvRecord {
            row_index,
            fields: fields.iter().map(|f| f.map(|s| s.to_string())).collect(),
        }
    }

    #[test]
    fn column_set_exact_match_passes() {
        let sidecar = SeedSidecar {
            materialization: SeedMaterialization::Table,
            columns: Some(vec![
                SidecarColumn {
                    name: "a".to_string(),
                    data_type: DataType::Text,
                    nullable: true,
                    description: None,
                },
                SidecarColumn {
                    name: "b".to_string(),
                    data_type: DataType::Text,
                    nullable: true,
                    description: None,
                },
            ]),
        };
        let result = validate_against_sidecar(
            Path::new("x.csv"),
            &["a".to_string(), "b".to_string()],
            &[],
            &sidecar,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn nullable_false_null_fails() {
        let sidecar = SeedSidecar {
            materialization: SeedMaterialization::Table,
            columns: Some(vec![SidecarColumn {
                name: "x".to_string(),
                data_type: DataType::Text,
                nullable: false,
                description: None,
            }]),
        };
        let rows = vec![make_record(1, &[None])];
        let err = validate_against_sidecar(Path::new("x.csv"), &["x".to_string()], &rows, &sidecar)
            .unwrap_err();
        assert!(matches!(
            err,
            ValidationError::NullInNonNullableColumn { .. }
        ));
    }

    #[test]
    fn integer_coercion_failure() {
        let sidecar = SeedSidecar {
            materialization: SeedMaterialization::Table,
            columns: Some(vec![SidecarColumn {
                name: "n".to_string(),
                data_type: DataType::Integer,
                nullable: true,
                description: None,
            }]),
        };
        let rows = vec![make_record(1, &[Some("abc")])];
        let err = validate_against_sidecar(Path::new("x.csv"), &["n".to_string()], &rows, &sidecar)
            .unwrap_err();
        assert!(matches!(err, ValidationError::CoercionFailed { .. }));
    }
}
