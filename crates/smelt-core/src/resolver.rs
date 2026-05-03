//! Kind-by-content resolver for smelt workspace entities.
//!
//! Per architecture.md §"Resolution":
//! - `.csv` → Seed (with optional sidecar YAML)
//! - `.yml` without sibling `.csv` → Source
//! - `.yml` with sibling `.csv` → sidecar (not addressable)
//! - `.sql` with bare SELECT → Model
//! - `.sql` with `smelt.define` → Function
//! - `.sql` with `smelt.test` → Test
//!
//! Address uniqueness is global across `paths:`. Collisions are hard errors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The kind of a project entity, determined by file format and content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    /// A bare-SELECT SQL file.
    Model,
    /// A `smelt.define` SQL file.
    Function,
    /// A `smelt.test` SQL file.
    Test,
    /// A `.csv` file, optionally paired with a sidecar `.yml`.
    Seed {
        /// Path to the sibling `.yml` file, if one exists.
        sidecar: Option<PathBuf>,
    },
    /// A standalone `.yml` file (no sibling `.csv`).
    Source,
}

/// A discovered project entity with its kind and address.
#[derive(Debug, Clone)]
pub struct ProjectEntity {
    /// The entity kind (Model, Function, Seed, Source, Test).
    pub kind: EntityKind,
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Address segments: the path from scan-root to leaf (without extension,
    /// without the scan-root prefix itself).
    ///
    /// For `paths: ["models"]` and `models/data/users.csv`, this is
    /// `["data", "users"]`.
    pub address_segments: Vec<String>,
    /// The scan root this entity was found under (e.g. `"models"`).
    pub scan_root: String,
}

/// Hard workspace-load errors produced by the resolver.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceLoadError {
    /// Two files resolve to the same `smelt.<path>` address.
    #[error(
        "duplicate address {}: found in {} and {}",
        address.join("."),
        path1.display(),
        path2.display()
    )]
    DuplicateAddress {
        /// The conflicting address segments (e.g. `["data", "users"]`).
        address: Vec<String>,
        /// First file that registered this address.
        path1: PathBuf,
        /// Second file that tried to register the same address.
        path2: PathBuf,
    },
    /// I/O error while walking the filesystem.
    #[error("I/O error while walking paths: {0}")]
    Io(#[from] std::io::Error),
}

/// Classify a single file by its format and content.
///
/// This is a pure function — it reads the file content when `content` is
/// `None` and needs to determine whether a `.sql` file is a model, function,
/// or test. The `dir_entries` slice (other files in the same directory) is
/// used to detect the `.csv`/`.yml` sidecar relationship.
///
/// # Arguments
/// * `path` — the file to classify (absolute path).
/// * `content` — pre-read file content, or `None` to read from disk.
/// * `sibling_paths` — other files in the same directory (for sidecar detection).
pub fn classify(
    path: &Path,
    content: Option<&str>,
    sibling_paths: &[PathBuf],
) -> Option<EntityKind> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    match ext {
        "csv" => {
            // Check if a sibling .yml exists (same stem, same directory).
            let sidecar = sibling_paths.iter().find(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("yml")
                    && p.file_stem().and_then(|s| s.to_str()) == Some(stem)
            });
            Some(EntityKind::Seed {
                sidecar: sidecar.cloned(),
            })
        }
        "yml" | "yaml" => {
            // If a sibling .csv exists (same stem), this is a sidecar — not addressable.
            let has_csv_sibling = sibling_paths.iter().any(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("csv")
                    && p.file_stem().and_then(|s| s.to_str()) == Some(stem)
            });
            if has_csv_sibling {
                None // sidecar — not addressable
            } else {
                Some(EntityKind::Source)
            }
        }
        "sql" => {
            // Read file content to determine what kind of SQL declaration it contains.
            let owned;
            let text = if let Some(c) = content {
                c
            } else {
                match std::fs::read_to_string(path) {
                    Ok(s) => {
                        owned = s;
                        &owned
                    }
                    Err(_) => return None,
                }
            };
            Some(classify_sql(text))
        }
        _ => None,
    }
}

/// Classify a `.sql` file by its content.
///
/// Precedence:
/// 1. Contains `smelt.define` → Function
/// 2. Contains `smelt.test` → Test
/// 3. Otherwise → Model (bare SELECT)
fn classify_sql(content: &str) -> EntityKind {
    // Quick string-based dispatch. We look for the literal token sequences
    // that the parser recognises.
    if content.contains("smelt.define") {
        EntityKind::Function
    } else if content.contains("smelt.test") {
        EntityKind::Test
    } else {
        EntityKind::Model
    }
}

/// Walk all `paths` under `project_root`, classify each file, compute
/// address segments (scan-root-stripped), and check for collisions.
///
/// Returns `Vec<ProjectEntity>` on success, or a `WorkspaceLoadError`
/// on the first collision detected.
pub fn walk_paths(
    project_root: &Path,
    paths: &[String],
) -> Result<Vec<ProjectEntity>, WorkspaceLoadError> {
    // address_segments → (file_path, scan_root) — collision detection.
    let mut seen: HashMap<Vec<String>, (PathBuf, String)> = HashMap::new();
    let mut entities: Vec<ProjectEntity> = Vec::new();

    for scan_root in paths {
        let root_dir = project_root.join(scan_root);
        if !root_dir.exists() {
            continue;
        }

        // First pass: collect all files under this scan root, grouped by
        // directory, so we can detect sibling relationships.
        let mut by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for entry in WalkDir::new(&root_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path().to_path_buf();
            let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            by_dir.entry(dir).or_default().push(path);
        }

        // Second pass: classify each file.
        for (dir, files) in &by_dir {
            let _ = dir; // directory used only for sibling lookup below
            for file_path in files {
                // Siblings are all other files in the same directory.
                let siblings: Vec<PathBuf> =
                    files.iter().filter(|p| *p != file_path).cloned().collect();

                let kind = match classify(file_path, None, &siblings) {
                    Some(k) => k,
                    None => continue, // sidecar or unrecognised extension
                };

                // Compute address segments: strip project_root + scan_root prefix,
                // then use parent dirs + stem.
                let rel = file_path
                    .strip_prefix(&root_dir)
                    .expect("file is under root_dir");

                let parent = rel.parent().unwrap_or(Path::new(""));
                let stem = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                let mut address_segments: Vec<String> = parent
                    .components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect();
                address_segments.push(stem);

                // Collision detection.
                if let Some((existing_path, _existing_root)) = seen.get(&address_segments) {
                    return Err(WorkspaceLoadError::DuplicateAddress {
                        address: address_segments,
                        path1: existing_path.clone(),
                        path2: file_path.clone(),
                    });
                }
                seen.insert(
                    address_segments.clone(),
                    (file_path.clone(), scan_root.clone()),
                );

                entities.push(ProjectEntity {
                    kind,
                    path: file_path.clone(),
                    address_segments,
                    scan_root: scan_root.clone(),
                });
            }
        }
    }

    Ok(entities)
}

/// Compute the default DB name for a persisted entity.
///
/// Per architecture.md §"Default materialization name mapping":
///   `smelt.staging.orders` → `main.staging_orders`
///   `smelt.users` → `main.users`
///
/// `address_segments` is the path tuple (e.g. `["staging", "orders"]`).
/// `target_schema` is the active target's `schema:` value (e.g. `"main"`).
pub fn default_db_name(address_segments: &[String], target_schema: &str) -> String {
    format!("{}.{}", target_schema, address_segments.join("_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classify_csv_no_sibling() {
        let path = PathBuf::from("/tmp/x/users.csv");
        let kind = classify(&path, None, &[]).unwrap();
        assert!(matches!(kind, EntityKind::Seed { sidecar: None }));
    }

    #[test]
    fn classify_csv_with_yml_sibling() {
        let path = PathBuf::from("/tmp/x/users.csv");
        let sidecar = PathBuf::from("/tmp/x/users.yml");
        let kind = classify(&path, None, std::slice::from_ref(&sidecar)).unwrap();
        match kind {
            EntityKind::Seed { sidecar: Some(s) } => assert_eq!(s, sidecar),
            other => panic!("expected Seed with sidecar, got {other:?}"),
        }
    }

    #[test]
    fn classify_yml_without_csv_sibling_is_source() {
        let path = PathBuf::from("/tmp/x/orders.yml");
        let kind = classify(&path, None, &[]).unwrap();
        assert_eq!(kind, EntityKind::Source);
    }

    #[test]
    fn classify_yml_with_csv_sibling_is_none() {
        let path = PathBuf::from("/tmp/x/orders.yml");
        let csv = PathBuf::from("/tmp/x/orders.csv");
        let kind = classify(&path, None, &[csv]);
        assert!(kind.is_none(), "sidecar yml should not produce an entity");
    }

    #[test]
    fn classify_sql_bare_select_is_model() {
        let path = PathBuf::from("/tmp/x/foo.sql");
        let kind = classify(&path, Some("SELECT id FROM t"), &[]).unwrap();
        assert_eq!(kind, EntityKind::Model);
    }

    #[test]
    fn classify_sql_define_is_function() {
        let path = PathBuf::from("/tmp/x/bar.sql");
        let kind = classify(
            &path,
            Some("smelt.define bar(x: Expr<Integer>) AS (x + 1)"),
            &[],
        )
        .unwrap();
        assert_eq!(kind, EntityKind::Function);
    }

    #[test]
    fn classify_sql_test_is_test() {
        let path = PathBuf::from("/tmp/x/check.sql");
        let kind = classify(&path, Some("smelt.test check_nulls AS (SELECT ...)"), &[]).unwrap();
        assert_eq!(kind, EntityKind::Test);
    }

    #[test]
    fn default_db_name_single_segment() {
        assert_eq!(
            default_db_name(&["users".to_string()], "main"),
            "main.users"
        );
    }

    #[test]
    fn default_db_name_multi_segment() {
        assert_eq!(
            default_db_name(&["staging".to_string(), "orders".to_string()], "main"),
            "main.staging_orders"
        );
    }
}
