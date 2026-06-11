use anyhow::{Context, Result};
use smelt_backend::Backend;
use smelt_core::resolver::default_db_name;
use smelt_core::seeds::{
    arrow::to_arrow_batches,
    csv::read_csv,
    infer::infer_columns,
    sidecar::{SeedMaterialization, SeedSidecar},
    validate::validate_against_sidecar,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

    /// Convert a `SeedInfo` (from `smelt_core::discover_seed_infos_strict`) into
    /// a `SeedFile` by attaching the target schema.
    ///
    /// This is the canonical conversion used by the CLI `seed` command after
    /// routing discovery through `smelt_core::discover_seed_infos_strict`
    /// (BUG-063 consolidation).
    pub fn from_seed_info(info: smelt_core::SeedInfo, schema: String) -> Self {
        SeedFile {
            name: info.name,
            path: info.path,
            address_segments: info.address_segments,
            schema,
            sidecar: info.sidecar,
        }
    }
}

pub struct SeedResult {
    pub name: String,
    pub qualified_name: String,
    pub row_count: usize,
    pub duration: Duration,
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

    /// BUG-063: `SeedFile::from_seed_info` correctly attaches the target schema
    /// and preserves all other fields from `SeedInfo`. This is the CLI-path
    /// assertion that the consolidation onto `smelt_core::discover_seed_infos_strict`
    /// produces correct `SeedFile` values.
    #[test]
    fn test_seed_file_from_seed_info_preserves_fields() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds").join("raw");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("users.csv"), "id,name\n1,alice\n").unwrap();

        let infos =
            smelt_core::discover_seed_infos_strict(tmp.path(), &["seeds".to_string()]).unwrap();
        assert_eq!(infos.len(), 1);

        let seed_file = SeedFile::from_seed_info(infos.into_iter().next().unwrap(), "prod".into());
        assert_eq!(seed_file.name, "users");
        assert_eq!(seed_file.address_segments, vec!["raw", "users"]);
        assert_eq!(seed_file.schema, "prod");
        assert_eq!(seed_file.qualified_name(), "prod.raw_users");
        assert_eq!(seed_file.table_name(), "raw_users");
    }

    /// BUG-028: selecting a source via `--select` must be a hard error, not a
    /// silent no-op that exits 0. Verify that `smelt_db::resolve_ref_path`
    /// returns `RefKind::Source` (not `RefKind::Seed`) for a source-YAML entity
    /// so that the kind-check added to `commands/seed.rs` fires correctly.
    ///
    /// This test does NOT call `run_seed` (that is an async binary command), but
    /// it exercises the exact DB query that the fix depends on: after
    /// `resolve_selector_args` resolves a selector to a canonical path, the
    /// command calls `resolve_ref_path` and rejects non-`Seed` kinds.
    #[test]
    fn test_select_source_resolves_as_source_not_seed() {
        use smelt_db::{resolve_ref_path, RefKind};
        use std::fs;
        use tempfile::TempDir;

        // Create a minimal hermetic project: smelt.yml + a source YAML file.
        let tmp = TempDir::new().unwrap();
        let proj = tmp.path();
        let models_dir = proj.join("models").join("sources").join("raw");
        fs::create_dir_all(&models_dir).unwrap();

        // A minimal per-entity source YAML (no CSV sibling → not a seed).
        // Only use spec-valid fields: `deny_unknown_fields` on RawSourceYaml rejects
        // legacy keys like `table:`, `schema:`, `database:`.
        fs::write(
            models_dir.join("customers.yml"),
            "columns:\n  - name: id\n    type: Integer\n",
        )
        .unwrap();

        // smelt.yml needed so set_project_input finds paths.
        fs::write(
            proj.join("smelt.yml"),
            "name: test_proj\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: dev.duckdb\n    schema: main\n",
        )
        .unwrap();

        // Build the same lightweight DB that `commands/seed.rs` uses.
        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(proj.to_path_buf(), String::new());
        db.set_workspace(vec![], vec![project]);
        let ws = db.workspace();

        // The source entity's canonical address segments = ["sources", "raw", "customers"].
        let source_path = vec![
            "sources".to_string(),
            "raw".to_string(),
            "customers".to_string(),
        ];

        let resolved = resolve_ref_path(&db, ws, source_path.clone()).expect(
            "sources.raw.customers must resolve in the lightweight DB (project_sources reads disk)",
        );

        // Assert it is a Source, not a Seed — the kind-check in commands/seed.rs
        // must fire and reject this selector with a hard error.
        assert_eq!(
            resolved.kind,
            RefKind::Source,
            "sources.raw.customers must resolve as Source, not {:?}",
            resolved.kind
        );

        // Confirm a seed path resolves as Seed (sanity-check the positive case).
        let seeds_dir = proj.join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(seeds_dir.join("products.csv"), "id,name\n1,Widget\n").unwrap();
        // Re-set project to pick up the new smelt.yml with seeds path.
        fs::write(
            proj.join("smelt.yml"),
            "name: test_proj\nversion: 1\npaths:\n  - models\n  - seeds\ntargets:\n  dev:\n    type: duckdb\n    database: dev.duckdb\n    schema: main\n",
        )
        .unwrap();
        // Rebuild DB so Salsa picks up the updated smelt.yml.
        let mut db2 = smelt_db::Database::default();
        let project2 = db2.set_project_input(proj.to_path_buf(), String::new());
        db2.set_workspace(vec![], vec![project2]);
        let ws2 = db2.workspace();

        let seed_path = vec!["products".to_string()];
        let seed_resolved = resolve_ref_path(&db2, ws2, seed_path.clone())
            .expect("products must resolve in DB after smelt.yml update");
        assert_eq!(
            seed_resolved.kind,
            RefKind::Seed,
            "products must resolve as Seed, not {:?}",
            seed_resolved.kind
        );
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
