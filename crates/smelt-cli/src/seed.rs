use anyhow::{Context, Result};
use smelt_backend::Backend;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedType {
    /// `seeds/<source_name>/<table>.csv` → loaded into source schema
    Source,
    /// `seeds/<table>.csv` → loaded into target schema
    Target,
}

#[derive(Debug, Clone)]
pub struct SeedFile {
    pub name: String,
    pub path: PathBuf,
    pub schema: String,
    pub seed_type: SeedType,
}

impl SeedFile {
    /// Fully qualified table name (schema.table)
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

pub struct SeedResult {
    pub name: String,
    pub qualified_name: String,
    pub row_count: usize,
    pub duration: Duration,
}

/// Discover seed CSV files in the configured seed paths.
///
/// Directory structure determines seed type:
/// - `seeds/<table>.csv` → Target seed (loaded into target_schema)
/// - `seeds/<source>/<table>.csv` → Source seed (loaded into <source> schema)
pub fn discover_seeds(
    project_root: &Path,
    seed_paths: &[String],
    target_schema: &str,
) -> Result<Vec<SeedFile>> {
    let mut seeds = Vec::new();

    for seed_path in seed_paths {
        let seed_dir = project_root.join(seed_path);
        if !seed_dir.exists() {
            continue;
        }

        discover_seeds_in_dir(&seed_dir, target_schema, &mut seeds)
            .with_context(|| format!("Failed to scan seed directory: {}", seed_dir.display()))?;
    }

    // Sort for deterministic ordering
    seeds.sort_by_key(|a| a.qualified_name());

    Ok(seeds)
}

fn discover_seeds_in_dir(
    seed_dir: &Path,
    target_schema: &str,
    seeds: &mut Vec<SeedFile>,
) -> Result<()> {
    for entry in std::fs::read_dir(seed_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "csv") {
            // Top-level CSV → target seed
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            seeds.push(SeedFile {
                name,
                path,
                schema: target_schema.to_string(),
                seed_type: SeedType::Target,
            });
        } else if path.is_dir() {
            // Subdirectory → source schema name
            let schema = path.file_name().unwrap().to_string_lossy().into_owned();

            for sub_entry in std::fs::read_dir(&path)? {
                let sub_entry = sub_entry?;
                let sub_path = sub_entry.path();

                if sub_path.is_file() && sub_path.extension().is_some_and(|ext| ext == "csv") {
                    let name = sub_path.file_stem().unwrap().to_string_lossy().into_owned();
                    seeds.push(SeedFile {
                        name,
                        path: sub_path,
                        schema: schema.clone(),
                        seed_type: SeedType::Source,
                    });
                }
            }
        }
    }

    Ok(())
}

/// Execute a single seed file: create schema, drop existing table, load CSV.
pub async fn execute_seed(
    backend: &dyn Backend,
    seed: &SeedFile,
    show_results: bool,
) -> Result<SeedResult> {
    let start = Instant::now();
    let qualified = seed.qualified_name();

    // 1. Ensure schema exists
    let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {}", seed.schema);
    backend
        .execute_sql(&create_schema)
        .await
        .with_context(|| format!("Failed to create schema '{}'", seed.schema))?;

    // 2. Drop existing table
    let drop_sql = format!("DROP TABLE IF EXISTS {}", qualified);
    backend
        .execute_sql(&drop_sql)
        .await
        .with_context(|| format!("Failed to drop table '{}'", qualified))?;

    // 3. Load CSV using DuckDB's read_csv_auto
    let abs_path = seed
        .path
        .canonicalize()
        .with_context(|| format!("Failed to resolve path: {}", seed.path.display()))?;
    let path_str = abs_path.display().to_string().replace('\'', "''");

    let create_sql = format!(
        "CREATE TABLE {} AS SELECT * FROM read_csv_auto('{}')",
        qualified, path_str
    );
    backend
        .execute_sql(&create_sql)
        .await
        .with_context(|| format!("Failed to load CSV into '{}'", qualified))?;

    // 4. Get row count
    let count_sql = format!("SELECT COUNT(*) AS cnt FROM {}", qualified);
    let batches = backend
        .execute_sql(&count_sql)
        .await
        .with_context(|| format!("Failed to count rows in '{}'", qualified))?;

    let row_count = if let Some(batch) = batches.first() {
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

    // 5. Show preview if requested
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

    Ok(SeedResult {
        name: seed.name.clone(),
        qualified_name: qualified,
        row_count,
        duration: start.elapsed(),
    })
}

/// Filter seeds by selector patterns.
/// Supports matching by table name or qualified name (schema.table).
pub fn filter_seeds(seeds: Vec<SeedFile>, selectors: &[String]) -> Vec<SeedFile> {
    seeds
        .into_iter()
        .filter(|seed| {
            selectors
                .iter()
                .any(|sel| seed.name == *sel || seed.qualified_name() == *sel)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_target_seeds() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("my_table.csv"), "id,name\n1,a\n").unwrap();

        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "my_table");
        assert_eq!(result[0].schema, "main");
        assert_eq!(result[0].seed_type, SeedType::Target);
    }

    #[test]
    fn test_discover_source_seeds() {
        let tmp = TempDir::new().unwrap();
        let raw_dir = tmp.path().join("seeds").join("raw");
        fs::create_dir_all(&raw_dir).unwrap();
        fs::write(raw_dir.join("users.csv"), "id,name\n1,a\n").unwrap();

        let result = discover_seeds(tmp.path(), &["seeds".to_string()], "main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "users");
        assert_eq!(result[0].schema, "raw");
        assert_eq!(result[0].seed_type, SeedType::Source);
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
        assert_eq!(result[1].qualified_name(), "raw.events");
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
                schema: "raw".to_string(),
                seed_type: SeedType::Source,
            },
            SeedFile {
                name: "events".to_string(),
                path: PathBuf::from("seeds/raw/events.csv"),
                schema: "raw".to_string(),
                seed_type: SeedType::Source,
            },
        ];

        let filtered = filter_seeds(seeds, &["users".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "users");
    }

    #[test]
    fn test_filter_seeds_by_qualified_name() {
        let seeds = vec![
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/raw/users.csv"),
                schema: "raw".to_string(),
                seed_type: SeedType::Source,
            },
            SeedFile {
                name: "users".to_string(),
                path: PathBuf::from("seeds/users.csv"),
                schema: "main".to_string(),
                seed_type: SeedType::Target,
            },
        ];

        let filtered = filter_seeds(seeds, &["raw.users".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].schema, "raw");
    }
}
