use anyhow::{Context, Result};
use smelt_backend::Backend;
use smelt_core::resolver::default_db_name;
use smelt_core::seeds::{
    arrow::to_arrow_batches,
    csv::read_csv,
    infer::infer_columns,
    sidecar::{parse_sidecar, SeedMaterialization, SeedSidecar},
    validate::validate_against_sidecar,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

/// A discovered seed CSV file.
#[derive(Debug, Clone)]
pub struct SeedFile {
    /// Leaf name (filename stem without extension).
    pub name: String,
    /// Absolute path to the CSV file on disk.
    pub path: PathBuf,
    /// Address segments from the scan-root to the leaf.
    /// For `seeds/data/users.csv` under `paths: ["seeds"]`,
    /// this is `["data", "users"]`.
    pub address_segments: Vec<String>,
    /// Target schema (from `smelt.yml`).
    pub schema: String,
    /// Parsed sidecar YAML, if present.
    pub sidecar: Option<SeedSidecar>,
}

impl SeedFile {
    /// Returns `true` when this seed has `materialization: ephemeral`.
    pub fn is_ephemeral(&self) -> bool {
        self.sidecar
            .as_ref()
            .map(|sc| sc.materialization == SeedMaterialization::Ephemeral)
            .unwrap_or(false)
    }
}

impl SeedFile {
    /// Fully qualified table name (`schema.table`), using the default
    /// DB-name mapping: path segments joined with `_`.
    pub fn qualified_name(&self) -> String {
        default_db_name(&self.address_segments, &self.schema)
    }

    /// Table name portion only (address segments joined with `_`).
    pub fn table_name(&self) -> String {
        self.address_segments.join("_")
    }

    /// Read and return the sidecar for this seed's CSV path, if one exists.
    /// Silently returns None on parse failure.
    fn read_sidecar_from_path(csv_path: &Path) -> Option<SeedSidecar> {
        let yml = csv_path.with_extension("yml");
        if yml.exists() {
            parse_sidecar(&yml).ok()
        } else {
            None
        }
    }
}

pub struct SeedResult {
    pub name: String,
    pub qualified_name: String,
    pub row_count: usize,
    pub duration: Duration,
}

/// Discover seed CSV files under the configured paths.
///
/// Per Phase 2: seeds are CSV files anywhere in the configured paths.
/// The address is the path from the scan-root to the leaf (no schema
/// prefix from directory name). The DB location is
/// `<target_schema>.<address_segments.join("_")>`.
pub fn discover_seeds(
    project_root: &Path,
    paths: &[String],
    target_schema: &str,
) -> Result<Vec<SeedFile>> {
    let mut seeds = Vec::new();

    for seed_path in paths {
        let seed_dir = project_root.join(seed_path);
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
            if path.extension().is_some_and(|ext| ext == "csv") {
                let name = path
                    .file_stem()
                    .expect("CSV file always has a stem")
                    .to_string_lossy()
                    .into_owned();

                // Compute address_segments: path from scan-root to leaf.
                let rel = path
                    .strip_prefix(&seed_dir)
                    .expect("path is under seed_dir");
                let parent = rel.parent().unwrap_or(std::path::Path::new(""));
                let mut address_segments: Vec<String> = parent
                    .components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect();
                address_segments.push(name.clone());

                // Read sidecar YAML if present.
                let sidecar = SeedFile::read_sidecar_from_path(&path);

                seeds.push(SeedFile {
                    name,
                    path,
                    address_segments,
                    schema: target_schema.to_string(),
                    sidecar,
                });
            }
        }
    }

    // Sort for deterministic ordering
    seeds.sort_by_key(|a| a.qualified_name());

    Ok(seeds)
}

/// Execute a single seed file: create schema, drop existing table, load CSV.
///
/// Returns `None` when the seed is ephemeral (nothing to load).
/// Returns `Some(SeedResult)` when the seed was loaded successfully.
///
/// Seeds with `materialization: view` or `materialized_view` produce a hard error.
pub async fn execute_seed(
    backend: &dyn Backend,
    seed: &SeedFile,
    show_results: bool,
) -> Result<Option<SeedResult>> {
    // Phase 5: check materialization before doing any work.
    if let Some(sc) = &seed.sidecar {
        match sc.materialization {
            SeedMaterialization::Ephemeral => {
                // Ephemeral seeds are inlined at compile time; no table to create.
                return Ok(None);
            }
            SeedMaterialization::View => {
                anyhow::bail!(
                    "Seed '{}': materialization 'view' is not supported for seeds; use 'table' or 'ephemeral'",
                    seed.qualified_name()
                );
            }
            SeedMaterialization::MaterializedView => {
                anyhow::bail!(
                    "Seed '{}': materialization 'materialized_view' is not supported for seeds; use 'table' or 'ephemeral'",
                    seed.qualified_name()
                );
            }
            SeedMaterialization::Table => {} // default — fall through
        }
    }

    let start = Instant::now();
    let qualified = seed.qualified_name();
    let table = seed.table_name();

    // 1. Ensure schema exists
    let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {}", seed.schema);
    backend
        .execute_sql(&create_schema)
        .await
        .with_context(|| format!("Failed to create schema '{}'", seed.schema))?;

    // 2. Parse CSV → validate against sidecar → infer/pin types → produce Arrow batches.
    let (headers, rows_iter) = read_csv(&seed.path)
        .with_context(|| format!("Failed to parse CSV: {}", seed.path.display()))?;
    let rows: Vec<_> = rows_iter
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("CSV parse error in: {}", seed.path.display()))?;

    // Phase 5: validate against sidecar (column-set match, nullable, type coercion).
    if let Some(sc) = &seed.sidecar {
        validate_against_sidecar(&seed.path, &headers, &rows, sc)
            .with_context(|| format!("Sidecar validation failed for '{}'", qualified))?;
    }

    // Determine column types: sidecar-pinned if available, otherwise infer.
    let col_types = if let Some(sc) = &seed.sidecar {
        if let Some(cols) = &sc.columns {
            // Build column order from CSV headers, using pinned types for known names.
            let pinned: std::collections::HashMap<&str, &smelt_types::DataType> = cols
                .iter()
                .map(|c| (c.name.as_str(), &c.data_type))
                .collect();
            headers
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    if let Some(&dt) = pinned.get(h.as_str()) {
                        (h.clone(), dt.clone())
                    } else {
                        // Fallback: infer this column from data.
                        let vals: Vec<_> = rows
                            .iter()
                            .filter_map(|r| r.fields.get(i).and_then(|v| v.as_deref()))
                            .collect();
                        (
                            h.clone(),
                            smelt_core::seeds::infer::infer_type_from_values_pub(&vals),
                        )
                    }
                })
                .collect()
        } else {
            // No column declarations — runtime inference (all rows).
            infer_columns(&rows, &headers, None)
        }
    } else {
        // No sidecar — runtime inference (all rows).
        infer_columns(&rows, &headers, None)
    };

    let (arrow_schema, batches) = to_arrow_batches(&seed.path, &col_types, &rows)
        .with_context(|| format!("Failed to build Arrow batches for '{}'", qualified))?;

    backend
        .load_table(&seed.schema, &table, arrow_schema, batches)
        .await
        .with_context(|| format!("Failed to load table '{}'", qualified))?;

    // 3. Get row count
    let count_sql = format!("SELECT COUNT(*) AS cnt FROM {}", qualified);
    let count_batches = backend
        .execute_sql(&count_sql)
        .await
        .with_context(|| format!("Failed to count rows in '{}'", qualified))?;

    let row_count = if let Some(batch) = count_batches.first() {
        use arrow::array::Array;
        let col = batch.column(0);
        if let Some(arr) = col.as_any().downcast_ref::<arrow::array::Int64Array>() {
            arr.value(0) as usize
        } else if let Some(arr) = col.as_any().downcast_ref::<arrow::array::UInt64Array>() {
            arr.value(0) as usize
        } else {
            0
        }
    } else {
        0
    };

    // 4. Show preview if requested
    if show_results {
        let preview_sql = format!("SELECT * FROM {} LIMIT 5", qualified);
        let preview = backend
            .execute_sql(&preview_sql)
            .await
            .with_context(|| format!("Failed to preview '{}'", qualified))?;
        if !preview.is_empty() {
            println!("\n  Preview:");
            arrow::util::pretty::print_batches(&preview)
                .with_context(|| "Failed to print preview")?;
            println!();
        }
    }

    Ok(Some(SeedResult {
        name: seed.name.clone(),
        qualified_name: qualified,
        row_count,
        duration: start.elapsed(),
    }))
}

/// Filter seeds by selector patterns.
/// Supports matching by leaf name, address dot-path, or qualified DB name.
pub fn filter_seeds(seeds: Vec<SeedFile>, selectors: &[String]) -> Vec<SeedFile> {
    seeds
        .into_iter()
        .filter(|seed| {
            selectors.iter().any(|sel| {
                // Match by leaf name
                seed.name == *sel
                    // Match by address path (e.g. "data.users")
                    || seed.address_segments.join(".") == *sel
                    // Match by qualified DB name (e.g. "main.data_users")
                    || seed.qualified_name() == *sel
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_seeds_top_level() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("my_table.csv"), "id,name\n1,a\n").unwrap();

        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "my_table");
        assert_eq!(result[0].address_segments, vec!["my_table"]);
        assert_eq!(result[0].schema, "main");
        assert_eq!(result[0].qualified_name(), "main.my_table");
    }

    #[test]
    fn test_discover_seeds_nested() {
        let tmp = TempDir::new().unwrap();
        let raw_dir = tmp.path().join("seeds").join("raw");
        fs::create_dir_all(&raw_dir).unwrap();
        fs::write(raw_dir.join("users.csv"), "id,name\n1,a\n").unwrap();

        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "users");
        assert_eq!(result[0].address_segments, vec!["raw", "users"]);
        assert_eq!(result[0].schema, "main");
        // DB name: main.raw_users (segments joined with _)
        assert_eq!(result[0].qualified_name(), "main.raw_users");
    }

    #[test]
    fn test_discover_mixed_seeds() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        let raw_dir = seeds_dir.join("raw");
        fs::create_dir_all(&raw_dir).unwrap();
        fs::write(seeds_dir.join("lookup.csv"), "id,val\n1,x\n").unwrap();
        fs::write(raw_dir.join("events.csv"), "id,type\n1,click\n").unwrap();

        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert_eq!(result.len(), 2);
        // Sorted by qualified name
        assert_eq!(result[0].qualified_name(), "main.lookup");
        assert_eq!(result[1].qualified_name(), "main.raw_events");
    }

    #[test]
    fn test_discover_no_seeds_dir() {
        let tmp = TempDir::new().unwrap();
        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_seeds_by_name() {
        let seeds = vec![
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/raw/users.csv"),
                address_segments: vec!["raw".to_string(), "users".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
            SeedFile {
                name: "events".to_string(),
                path: PathBuf::from("seeds/raw/events.csv"),
                address_segments: vec!["raw".to_string(), "events".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
        ];

        let filtered = filter_seeds(seeds, &["users".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "users");
    }

    #[test]
    fn test_filter_seeds_by_address_path() {
        let seeds = vec![
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/raw/users.csv"),
                address_segments: vec!["raw".to_string(), "users".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/staging/users.csv"),
                address_segments: vec!["staging".to_string(), "users".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
        ];

        let filtered = filter_seeds(seeds, &["raw.users".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].address_segments, vec!["raw", "users"]);
    }

    #[test]
    fn test_filter_seeds_by_qualified_name() {
        let seeds = vec![
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/raw/users.csv"),
                address_segments: vec!["raw".to_string(), "users".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/users.csv"),
                address_segments: vec!["users".to_string()],
                schema: "main".to_string(),
                sidecar: None,
            },
        ];

        let filtered = filter_seeds(seeds, &["main.raw_users".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].address_segments, vec!["raw", "users"]);
    }
}
