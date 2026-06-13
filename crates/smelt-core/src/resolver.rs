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
//!
//! ## Address authority
//!
//! `resolve_address_map` is the single authority for `smelt.<path>` address
//! uniqueness. It operates on post-discovery descriptor sets (already have
//! `address_segments` computed) so CLI ↔ LSP parity holds by construction.

use crate::discovery::ModelFile;
use crate::seeds::SeedInfo;
use crate::sources::SourceInfo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// An aggregate `sources.yml` / `sources.yaml` was found at the project
    /// root, which is no longer supported. Sources must be per-entity `.yml`
    /// files placed under one of the project's `paths:` directories.
    ///
    /// Migration: for each `sources.<schema>.tables.<table>`, create a file
    /// at `<paths[0]>/sources/<schema>/<table>.yml` with the same columns.
    #[error(
        "Aggregate sources file not supported: {}\n\
         \n\
         Sources must now be per-entity YAML files placed under one of the project's\n\
         `paths:` directories (e.g. `models/sources/raw/users.yml`).\n\
         \n\
         Migration: for each `sources.<schema>.tables.<table>` entry, create:\n\
           <paths[0]>/sources/<schema>/<table>.yml\n\
         with a `columns:` key and the same column definitions.",
        path.display()
    )]
    AggregateSourcesYmlNotSupported {
        /// Path to the offending aggregate file.
        path: PathBuf,
    },
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
            // `smelt.yml` / `smelt.yaml` are project config files, not sources.
            if stem == "smelt" {
                return None;
            }
            // `sources.yml` / `sources.yaml` is the aggregate sources file (legacy);
            // it is not addressable as a per-entity source.
            if stem == "sources" {
                return None;
            }
            // If a sibling .csv exists (same stem), this is a sidecar — not addressable.
            let has_csv_sibling = sibling_paths.iter().any(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("csv")
                    && p.file_stem().and_then(|s| s.to_str()) == Some(stem)
            });
            if has_csv_sibling {
                return None; // sidecar — not addressable
            }
            // Content-sniff: a per-entity source YAML is a top-level mapping
            // with at least one known source-schema key. Arbitrary data YAML
            // files (lists, mappings with domain-specific keys) are NOT sources.
            let text = if let Some(c) = content {
                c.to_string()
            } else {
                match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => return None,
                }
            };
            if looks_like_source_yaml(&text) {
                Some(EntityKind::Source)
            } else {
                None
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

/// Return `true` if a YAML file's content looks like a per-entity source
/// definition. A source YAML is a top-level YAML mapping containing at least
/// one known source-schema key (`description`, `columns`, `name`,
/// `materialization`, `tags`, `timeseries`). This distinguishes source
/// definitions from arbitrary data files used by `smelt.config.load_yaml`
/// (which may be lists or mappings with domain-specific keys).
fn looks_like_source_yaml(text: &str) -> bool {
    const SOURCE_KEYS: &[&str] = &[
        "description:",
        "columns:",
        "materialization:",
        "tags:",
        "timeseries:",
    ];
    // Must not start with `-` (list item marker) — source YAMLs are mappings.
    let first_non_empty = text.trim_start();
    if first_non_empty.starts_with('-') {
        return false;
    }
    // Must have at least one known source key at the top level.
    // Line-based scan: look for a key at column 0 (no leading whitespace).
    text.lines().any(|line| {
        SOURCE_KEYS
            .iter()
            .any(|k| line == *k || line.starts_with(k))
    })
}

// ---------------------------------------------------------------------------
// Address-map authority (BUG-002, BUG-021)
// ---------------------------------------------------------------------------

/// The kind of entity in an address-map or collision report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityRefKind {
    /// A SQL model or function file.
    SqlModel,
    /// A seed CSV file.
    Seed,
    /// A source YAML file.
    Source,
}

/// A reference to a discovered workspace entity with enough information to
/// produce a useful collision diagnostic.
#[derive(Debug, Clone)]
pub struct EntityRef {
    /// The entity kind.
    pub kind: EntityRefKind,
    /// Primary file path on disk.
    pub path: PathBuf,
    /// The canonical address segments (`address_segments.join(".")` is the key).
    pub address_segments: Vec<String>,
}

/// A collision between two entities that both claim the same `smelt.<path>` address.
#[derive(Debug, Clone)]
pub struct AddressCollision {
    /// The conflicting address segments.
    pub address: Vec<String>,
    /// The first entity to claim this address.
    pub first: EntityRef,
    /// The second entity that collided with `first`.
    pub second: EntityRef,
}

/// Compute the canonical address map and collision list across all entity kinds.
///
/// This is the **single authority** for `smelt.<path>` address uniqueness per
/// architecture.md §"Workspace loading parity rule". It operates on the
/// post-discovery descriptor sets (callers have already computed
/// `address_segments`) so CLI ↔ LSP parity holds by construction.
///
/// Returns `(address_map, collisions)`:
/// - `address_map`: maps `address_segments.join(".")` → the first `EntityRef`
///   that claimed it.
/// - `collisions`: one entry per address claimed by two or more entities.
///
/// Entities with empty `address_segments` are silently skipped (the
/// scan-root could not be determined — they cannot be addressed).
pub fn resolve_address_map(
    sql_files: &[ModelFile],
    seeds: &[SeedInfo],
    sources: &[SourceInfo],
) -> (HashMap<String, EntityRef>, Vec<AddressCollision>) {
    let mut map: HashMap<String, EntityRef> = HashMap::new();
    let mut collisions: Vec<AddressCollision> = Vec::new();

    fn register(
        map: &mut HashMap<String, EntityRef>,
        collisions: &mut Vec<AddressCollision>,
        kind: EntityRefKind,
        path: PathBuf,
        segments: Vec<String>,
    ) {
        if segments.is_empty() {
            return;
        }
        let key = segments.join(".");
        let new_ref = EntityRef {
            kind,
            path,
            address_segments: segments.clone(),
        };
        match map.entry(key) {
            std::collections::hash_map::Entry::Occupied(occ) => {
                collisions.push(AddressCollision {
                    address: segments,
                    first: occ.get().clone(),
                    second: new_ref,
                });
            }
            std::collections::hash_map::Entry::Vacant(vac) => {
                vac.insert(new_ref);
            }
        }
    }

    for model in sql_files {
        register(
            &mut map,
            &mut collisions,
            EntityRefKind::SqlModel,
            model.path.clone(),
            model.address_segments.clone(),
        );
    }
    for seed in seeds {
        register(
            &mut map,
            &mut collisions,
            EntityRefKind::Seed,
            seed.path.clone(),
            seed.address_segments.clone(),
        );
    }
    for source in sources {
        register(
            &mut map,
            &mut collisions,
            EntityRefKind::Source,
            source.path.clone(),
            source.address_segments.clone(),
        );
    }

    (map, collisions)
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
        let source_content = "description: Raw orders\ncolumns:\n  - name: id\n    type: INTEGER\n";
        let kind = classify(&path, Some(source_content), &[]).unwrap();
        assert_eq!(kind, EntityKind::Source);
    }

    #[test]
    fn classify_yml_list_content_is_none() {
        // A YAML file whose content is a list (not a mapping) is not a source.
        let path = PathBuf::from("/tmp/x/cohorts.yaml");
        let list_content = "- name: us_west\n  region: us-west-2\n";
        assert!(classify(&path, Some(list_content), &[]).is_none());
    }

    #[test]
    fn classify_yml_arbitrary_mapping_is_none() {
        // A YAML mapping with no recognized source-schema keys is not a source.
        let path = PathBuf::from("/tmp/x/tenants.yaml");
        let arbitrary_content =
            "tenant_a:\n  host: db-a.example.com\ntenant_b:\n  host: db-b.example.com\n";
        assert!(classify(&path, Some(arbitrary_content), &[]).is_none());
    }

    #[test]
    fn classify_smelt_yml_is_none() {
        // `smelt.yml` is the project config file, not a source.
        let path = PathBuf::from("/tmp/x/smelt.yml");
        assert!(classify(&path, None, &[]).is_none());
        let path_yaml = PathBuf::from("/tmp/x/smelt.yaml");
        assert!(classify(&path_yaml, None, &[]).is_none());
    }

    #[test]
    fn classify_sources_yml_is_none() {
        // `sources.yml` is the aggregate sources file, not a per-entity source.
        let path = PathBuf::from("/tmp/x/sources.yml");
        assert!(classify(&path, None, &[]).is_none());
        let path_yaml = PathBuf::from("/tmp/x/sources.yaml");
        assert!(classify(&path_yaml, None, &[]).is_none());
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
