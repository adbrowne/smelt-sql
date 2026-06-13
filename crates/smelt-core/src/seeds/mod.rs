//! Seed discovery and CSV type inference for smelt.
//!
//! Submodules:
//! - `csv` — strict CSV reader (comma delimiter, double-quote quoting, BOM-stripping)
//! - `infer` — type inferencer producing `DataType` from CSV value samples
//! - `arrow` — Arrow `RecordBatch` builder from parsed rows + inferred types
//! - `error` — `SeedError` type
//! - `sidecar` — sidecar YAML parsing (column declarations, materialization)
//! - `validate` — validation of CSV data against a sidecar YAML
//! - `ephemeral` — `VALUES (…)` CTE builder for ephemeral seeds

pub mod arrow;
pub mod csv;
pub mod ephemeral;
pub mod error;
pub mod infer;
pub mod sidecar;
pub mod validate;

pub use error::SeedError;
pub use sidecar::{parse_sidecar, SeedMaterialization, SeedSidecar, SidecarColumn};
pub use validate::{validate_against_sidecar, ValidationError};

use crate::discovery::ModelDiscovery;
use anyhow::Context;
use infer::infer_columns;
use smelt_types::DataType;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Information about a seed CSV file discovered in the project's seed directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedInfo {
    /// CSV filename without extension (leaf name, used for simple lookups)
    pub name: String,
    /// Absolute path to the CSV file
    pub path: PathBuf,
    /// Column names and inferred (or pinned from sidecar) types
    pub columns: Vec<(String, DataType)>,
    /// Full address segments from the scan-root to the leaf name (without
    /// extension). For `seeds/data/users.csv` under `paths: ["seeds"]`,
    /// this is `["data", "users"]`. For a top-level seed `seeds/users.csv`,
    /// this is `["users"]`.
    pub address_segments: Vec<String>,
    /// Parsed sidecar YAML, if a sibling `.yml` file exists.
    ///
    /// `None` when no sidecar is present (full inference).
    pub sidecar: Option<SeedSidecar>,
}

impl SeedInfo {
    /// Returns `true` when this seed's materialization is `ephemeral`.
    pub fn is_ephemeral(&self) -> bool {
        self.sidecar
            .as_ref()
            .map(|sc| sc.materialization == SeedMaterialization::Ephemeral)
            .unwrap_or(false)
    }

    /// The CTE alias name used when this seed is inlined as an ephemeral.
    /// Follows the `__smelt_<address_segments.join("_")>` convention.
    pub fn cte_alias(&self) -> String {
        format!("__smelt_{}", self.address_segments.join("_"))
    }
}

/// Discover seed CSV files in the project's configured paths and infer column types.
///
/// Seeds are CSV files under the configured `paths:` directories. This
/// function walks recursively and populates `address_segments` with the
/// path from the scan-root to the leaf (e.g. `["data", "users"]` for
/// `seeds/data/users.csv` under `paths: ["seeds"]`).
///
/// `paths` is the unified `paths:` list from `smelt.yml`.
///
/// Sibling sidecar `.yml` files are silently ignored in this variant —
/// use `discover_seed_infos_with_sidecars` when sidecar semantics matter.
pub fn discover_seed_infos(project_dir: &Path, paths: &[String]) -> Vec<SeedInfo> {
    discover_seed_infos_impl(project_dir, paths, false)
}

/// Like `discover_seed_infos`, but also reads any sibling `.yml` sidecar file
/// and stores the parsed `SeedSidecar` in `SeedInfo::sidecar`.
///
/// When a sidecar cannot be parsed (bad YAML, unknown type), the error is
/// silently ignored and `sidecar` is `None` — the LSP will surface the error
/// as a diagnostic via a separate code path.
pub fn discover_seed_infos_with_sidecars(project_dir: &Path, paths: &[String]) -> Vec<SeedInfo> {
    discover_seed_infos_impl(project_dir, paths, true)
}

/// Strict variant of seed discovery for CLI use.
///
/// Unlike `discover_seed_infos_with_sidecars`, a sidecar that fails to parse
/// (forbidden `name:` key, malformed YAML, unknown column type) aborts
/// discovery with an error rather than being silently dropped. Used by the
/// CLI `smelt seed` command where a malformed sidecar is a hard stop
/// (`seeds.md` Invariant 8 / BUG-027 regression contract).
pub fn discover_seed_infos_strict(
    project_dir: &Path,
    paths: &[String],
) -> anyhow::Result<Vec<SeedInfo>> {
    let mut seeds = Vec::new();

    for seed_path in paths {
        let seed_dir = project_dir.join(seed_path);
        if !seed_dir.exists() {
            continue;
        }

        for entry in WalkDir::new(&seed_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path().to_path_buf();
            if path.extension().is_some_and(|e| e == "csv") {
                let name = path
                    .file_stem()
                    .expect("CSV file always has a stem")
                    .to_string_lossy()
                    .into_owned();

                // Address via the D-01 strip-list rule (shared with models/sources).
                let address_segments =
                    ModelDiscovery::compute_address_segments(&path, project_dir, paths);

                // Strict sidecar reading: propagate parse errors (unlike the
                // lenient LSP path which silently ignores them).
                let yml_path = path.with_extension("yml");
                let sidecar =
                    if yml_path.exists() {
                        Some(parse_sidecar(&yml_path).with_context(|| {
                            format!("Invalid seed sidecar {}", yml_path.display())
                        })?)
                    } else {
                        None
                    };

                let columns = if let Some(sc) = &sidecar {
                    if let Some(cols) = &sc.columns {
                        infer_csv_columns_with_pins(&path, cols)
                    } else {
                        infer_csv_columns(&path)
                    }
                } else {
                    infer_csv_columns(&path)
                };

                seeds.push(SeedInfo {
                    name,
                    path,
                    columns,
                    address_segments,
                    sidecar,
                });
            }
        }
    }

    seeds.sort_by(|a, b| a.address_segments.cmp(&b.address_segments));
    Ok(seeds)
}

/// Internal implementation shared by `discover_seed_infos` and
/// `discover_seed_infos_with_sidecars`.
fn discover_seed_infos_impl(
    project_dir: &Path,
    paths: &[String],
    read_sidecars: bool,
) -> Vec<SeedInfo> {
    use crate::discovery::project_root_files_by_dir;
    use crate::resolver::{classify, EntityKind};

    let mut seeds = Vec::new();

    // Universal walk (D-01): project-wide discovery, not gated on `paths:`.
    for (_, files) in project_root_files_by_dir(project_dir) {
        for file_path in &files {
            // Only process CSV files classified as seeds.
            if !matches!(
                classify(file_path, None, &files),
                Some(EntityKind::Seed { .. })
            ) {
                continue;
            }

            let name = file_path
                .file_stem()
                .expect("CSV file always has a stem")
                .to_string_lossy()
                .into_owned();

            // Address via the D-01 strip-list rule (shared with models/sources).
            let address_segments =
                ModelDiscovery::compute_address_segments(file_path, project_dir, paths);

            // Try to read the sidecar YAML (same stem, same directory, .yml extension).
            let sidecar = if read_sidecars {
                let yml_path = file_path.with_extension("yml");
                if yml_path.exists() {
                    parse_sidecar(&yml_path).ok()
                } else {
                    None
                }
            } else {
                None
            };

            // Use sidecar-pinned columns when available and valid;
            // otherwise fall back to inference.
            let columns = if let Some(sc) = &sidecar {
                if let Some(cols) = &sc.columns {
                    // Reorder columns to match the CSV header order.
                    infer_csv_columns_with_pins(file_path, cols)
                } else {
                    infer_csv_columns(file_path)
                }
            } else {
                infer_csv_columns(file_path)
            };

            seeds.push(SeedInfo {
                name,
                path: file_path.clone(),
                columns,
                address_segments,
                sidecar,
            });
        }
    }

    seeds.sort_by(|a, b| a.address_segments.cmp(&b.address_segments));
    seeds
}

/// Parse CSV headers and infer column types from the first 100 data rows.
///
/// Uses the spec-compliant inferencer (via `infer_columns` with
/// `sample_limit = Some(100)`) for consistent behaviour with the runtime path.
fn infer_csv_columns(path: &Path) -> Vec<(String, DataType)> {
    // Use the strict csv reader; fall back to empty on any error (LSP tolerance).
    match csv::read_csv(path) {
        Err(_) => Vec::new(),
        Ok((headers, rows_iter)) => {
            let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
            infer_columns(&rows, &headers, Some(100))
        }
    }
}

/// Use sidecar-pinned column types. Returns columns in CSV header order,
/// with pinned types applied; any header column not in the sidecar falls
/// back to inference (should only happen during mismatch, which is caught
/// at validation time separately).
fn infer_csv_columns_with_pins(
    path: &Path,
    sidecar_cols: &[SidecarColumn],
) -> Vec<(String, DataType)> {
    match csv::read_csv(path) {
        Err(_) => Vec::new(),
        Ok((headers, rows_iter)) => {
            let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
            // Build a map from sidecar column name → DataType.
            let pinned: std::collections::HashMap<&str, &DataType> = sidecar_cols
                .iter()
                .map(|c| (c.name.as_str(), &c.data_type))
                .collect();

            // For each header, use the pinned type if present, otherwise infer.
            headers
                .iter()
                .enumerate()
                .map(|(col_idx, header)| {
                    if let Some(&dt) = pinned.get(header.as_str()) {
                        (header.clone(), dt.clone())
                    } else {
                        // Fall back to infer for this column only.
                        let values: Vec<_> = rows
                            .iter()
                            .filter_map(|r| r.fields.get(col_idx).and_then(|v| v.as_deref()))
                            .collect();
                        (header.clone(), infer::infer_type_from_values_pub(&values))
                    }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_seed_infos_basic() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("order_statuses.csv"),
            "status_code,status_label,is_terminal,is_successful\ncompleted,Completed,true,true\ncancelled,Cancelled,true,false\n",
        ).unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].name, "order_statuses");
        assert_eq!(seeds[0].columns.len(), 4);

        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["status_code"], DataType::Text);
        assert_eq!(col_map["status_label"], DataType::Text);
        assert_eq!(col_map["is_terminal"], DataType::Boolean);
        assert_eq!(col_map["is_successful"], DataType::Boolean);
    }

    #[test]
    fn test_infer_numeric_types() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("numbers.csv"),
            "id,score,ratio\n1,100,0.5\n2,200,1.5\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["id"], DataType::Integer);
        assert_eq!(col_map["score"], DataType::Integer);
        // 0.5 and 1.5 → DECIMAL(2,1) within cap
        assert!(
            matches!(col_map["ratio"], DataType::Decimal { .. }),
            "ratio should be Decimal, got {:?}",
            col_map["ratio"]
        );
    }

    #[test]
    fn test_discover_no_seeds_dir() {
        let tmp = TempDir::new().unwrap();
        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_seed_date_column_infers_as_date() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("users.csv"),
            "user_id,user_name,signup_date\n1,Alice,2025-01-01\n2,Bob,2025-01-02\n3,Charlie,2025-01-03\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["user_id"], DataType::Integer);
        assert_eq!(col_map["user_name"], DataType::Text);
        assert_eq!(col_map["signup_date"], DataType::Date);
    }

    #[test]
    fn test_seed_timestamp_column_infers_as_timestamp() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,event_type,event_timestamp\n1,login,2025-01-10 08:00:00\n2,page_view,2025-01-10 08:05:00\n3,logout,2025-01-10 09:00:00\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["event_id"], DataType::Integer);
        assert_eq!(col_map["event_type"], DataType::Text);
        assert_eq!(
            col_map["event_timestamp"],
            DataType::Timestamp {
                with_timezone: false
            }
        );
    }

    #[test]
    fn test_seed_text_column_infers_as_text() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("notes.csv"),
            "note_id,body,almost_date\n1,hello world,2025-13-01\n2,another note,not-a-date\n3,third,2025-01-32\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["note_id"], DataType::Integer);
        assert_eq!(col_map["body"], DataType::Text);
        assert_eq!(col_map["almost_date"], DataType::Text);
    }

    /// BUG-063 regression: `discover_seed_infos_strict` must propagate sidecar
    /// parse errors (BUG-027 contract) — it is the canonical strict discovery
    /// path that replaced the CLI-local `discover_seeds` reimplementation.
    #[test]
    fn test_discover_seed_infos_strict_propagates_sidecar_error() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("users.csv"), "id,name\n1,alice\n").unwrap();
        // `name:` is forbidden on a seed sidecar — must be a hard error.
        fs::write(seeds_dir.join("users.yml"), "name: custom\n").unwrap();

        let result = discover_seed_infos_strict(tmp.path(), &["seeds".to_string()]);
        let err = result.expect_err("strict discovery must propagate sidecar errors");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("name:"),
            "error should explain the forbidden `name:` key, got: {msg}"
        );
    }

    #[test]
    fn test_discover_seed_infos_strict_unknown_materialization_is_error() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("t.csv"), "id\n1\n").unwrap();
        fs::write(seeds_dir.join("t.yml"), "materialization: bogus\n").unwrap();

        let result = discover_seed_infos_strict(tmp.path(), &["seeds".to_string()]);
        let err = result.expect_err("unknown materialization must abort strict discovery");
        assert!(format!("{err:#}").contains("bogus"));
    }

    #[test]
    fn test_discover_seed_infos_strict_happy_path() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        let nested = seeds_dir.join("raw");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("users.csv"), "id,name\n1,alice\n").unwrap();

        let result = discover_seed_infos_strict(tmp.path(), &["seeds".to_string()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].address_segments, vec!["raw", "users"]);
    }

    #[test]
    fn test_seed_t_separator_timestamp_falls_back_to_text() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,iso_timestamp\n1,2025-01-10T08:00:00\n2,2025-01-10T08:05:00\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["iso_timestamp"], DataType::Text);
    }

    #[test]
    fn test_seed_tz_suffix_timestamp_falls_back_to_text() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,zoned_ts,offset_ts\n1,2025-01-10 08:00:00Z,2025-01-10 08:00:00+00\n2,2025-01-10 08:05:00Z,2025-01-10 08:05:00-05\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["zoned_ts"], DataType::Text);
        assert_eq!(col_map["offset_ts"], DataType::Text);
    }

    /// A seed CSV in `billing/seeds/users.csv` (not in `paths: ["models"]`) must
    /// be discovered project-wide after D-01 universal walk.
    /// Spec: seeds.md §"What a seed is" — project-wide discovery.
    #[test]
    fn seed_discovered_project_wide() {
        let tmp = TempDir::new().unwrap();
        // paths: ["models"] — billing/ is NOT in paths
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        let billing_seeds = tmp.path().join("billing").join("seeds");
        fs::create_dir_all(&billing_seeds).unwrap();
        fs::write(billing_seeds.join("users.csv"), "id,name\n1,alice\n").unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["models".to_string()]);
        assert!(
            seeds.iter().any(|s| s.name == "users"),
            "users.csv outside models/ must be discovered project-wide; got: {:?}",
            seeds.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let users = seeds.iter().find(|s| s.name == "users").unwrap();
        assert_eq!(
            users.address_segments,
            vec!["billing", "seeds", "users"],
            "address must include full path since billing/ is not a paths prefix"
        );
    }
}
