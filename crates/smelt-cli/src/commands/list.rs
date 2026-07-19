//! `smelt list` — enumerate discovered project entities (offline).
//!
//! Walks `smelt_core::load_workspace` output (workspace-loading parity rule
//! — no private re-discovery), classifies each SQL declaration into a
//! model/test/check entity (function declarations are not listed — they are
//! not addressable `smelt.ref()` targets), and adds seeds and sources from
//! their own discovery entry points. `docs/specs/cli.md` §"`smelt list`" is
//! the correctness oracle for this command's surface.

use anyhow::{Context, Result};
use serde::Serialize;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    find_project_root, parse_selector, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_core::{discover_seed_infos, load_workspace, ModelFile};
use thiserror::Error;

use crate::ListArgs;

/// Errors specific to `smelt list`'s own classification of "usage error"
/// (`docs/specs/cli.md` §"Exit codes": `smelt list` exits `2` on a parse
/// error or an unresolvable/ambiguous selector). See [`exit_code_for`].
#[derive(Debug, Error)]
enum ListError {
    #[error("Parse errors in {} model(s): {}", .0.len(), .0.join(", "))]
    ParseErrors(Vec<String>),
    #[error("{0}")]
    UnresolvableSelector(String),
}

/// Classify a top-level command error for `smelt list`'s exit code: `2` for
/// [`ListError`] (parse errors, unresolvable selectors) or the shared
/// `ProjectError`/`ConfigError` usage errors; `1` otherwise. Same pattern as
/// `commands::init::exit_code_for`.
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<ListError>().is_some() {
        2
    } else {
        smelt_cli::exit_code_for(err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EntityKind {
    Model,
    Seed,
    Source,
    Test,
    Check,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EntityKind::Model => "model",
            EntityKind::Seed => "seed",
            EntityKind::Source => "source",
            EntityKind::Test => "test",
            EntityKind::Check => "check",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize)]
struct ListEntry {
    address: String,
    kind: EntityKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    materialization: Option<String>,
}

/// Whether `model` is a `smelt.define`/`smelt.extern` function declaration —
/// not a listed entity (functions are not `smelt.ref()` targets).
fn is_function(model: &ModelFile) -> bool {
    use smelt_parser::ast::File as AstFile;
    let clean = smelt_parser::strip_frontmatter(&model.content);
    let parse = smelt_parser::parse(&clean);
    let Some(file) = AstFile::cast(parse.syntax()) else {
        return false;
    };
    file.defines().next().is_some() || file.externs().next().is_some()
}

pub async fn list(args: ListArgs, scope: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let loaded = load_workspace(&project_dir);

    let parse_error_models: Vec<String> = loaded
        .sql_files
        .iter()
        .filter(|m| !m.parse_errors.is_empty())
        .map(|m| m.name.clone())
        .collect();
    if !parse_error_models.is_empty() {
        return Err(ListError::ParseErrors(parse_error_models).into());
    }

    // Classify every discovered SQL declaration; functions are excluded.
    let mut model_files: Vec<ModelFile> = Vec::new();
    let mut test_entries: Vec<ListEntry> = Vec::new();
    let mut check_entries: Vec<ListEntry> = Vec::new();
    for model in &loaded.sql_files {
        if model.is_test() {
            test_entries.push(ListEntry {
                address: format!("smelt.{}", model.canonical_path()),
                kind: EntityKind::Test,
                materialization: None,
            });
        } else if model.is_check() {
            check_entries.push(ListEntry {
                address: format!("smelt.{}", model.canonical_path()),
                kind: EntityKind::Check,
                materialization: None,
            });
        } else if !is_function(model) {
            model_files.push(model.clone());
        }
    }

    // --select/--exclude narrow the model set, same selector surface as
    // `smelt run`/`smelt build` (`docs/specs/model_selection.md`).
    let selected_model_names: Option<std::collections::HashSet<String>> = if args.select.is_empty()
        && args.exclude.is_empty()
    {
        None
    } else {
        let sources = SourcesConfig::load(&project_dir).ok();
        let graph = DependencyGraph::build(model_files.clone(), sources.as_ref())
            .with_context(|| "Failed to build dependency graph")?;

        let mut db = smelt_db::Database::default();
        let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, &loaded);
        db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
        let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");

        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &loaded.config.paths, scope);

        let resolved_select = resolve_selector_args(
            &db,
            ws,
            ingested.project,
            active_scope.as_ref(),
            &args.select,
        )
        .map_err(|e| ListError::UnresolvableSelector(e.to_string()))?;
        let resolved_exclude = resolve_selector_args(
            &db,
            ws,
            ingested.project,
            active_scope.as_ref(),
            &args.exclude,
        )
        .map_err(|e| ListError::UnresolvableSelector(e.to_string()))?;

        let mut selected = if resolved_select.is_empty() {
            graph.all_model_names()
        } else {
            let selectors: Vec<_> = resolved_select
                .iter()
                .map(|s| {
                    parse_selector(s).map_err(|e| ListError::UnresolvableSelector(e.to_string()))
                })
                .collect::<Result<_, _>>()?;
            graph
                .select_models(&selectors, &loaded.config)
                .map_err(|e| ListError::UnresolvableSelector(e.to_string()))?
        };
        if !resolved_exclude.is_empty() {
            let selectors: Vec<_> = resolved_exclude
                .iter()
                .map(|s| {
                    parse_selector(s).map_err(|e| ListError::UnresolvableSelector(e.to_string()))
                })
                .collect::<Result<_, _>>()?;
            let excluded = graph
                .select_models(&selectors, &loaded.config)
                .map_err(|e| ListError::UnresolvableSelector(e.to_string()))?;
            selected.retain(|m| !excluded.contains(m));
        }
        Some(selected)
    };

    let mut model_entries: Vec<ListEntry> = model_files
        .iter()
        .filter(|m| {
            selected_model_names
                .as_ref()
                .is_none_or(|s| s.contains(&m.canonical_path()))
        })
        .map(|m| {
            let materialization = loaded
                .config
                .get_materialization_with_metadata(&m.canonical_path(), m.metadata.as_deref());
            ListEntry {
                address: format!("smelt.{}", m.canonical_path()),
                kind: EntityKind::Model,
                materialization: Some(format!("{:?}", materialization).to_lowercase()),
            }
        })
        .collect();

    // Seeds and sources are always listed in full — they are not part of
    // the model dependency graph `--select`/`--exclude` narrows.
    let mut seed_entries: Vec<ListEntry> = discover_seed_infos(&project_dir, &loaded.config.paths)
        .iter()
        .map(|seed| ListEntry {
            address: format!("smelt.{}", seed.address_segments.join(".")),
            kind: EntityKind::Seed,
            materialization: None,
        })
        .collect();

    let mut source_entries: Vec<ListEntry> = Vec::new();
    if let Ok(sources) = SourcesConfig::load(&project_dir) {
        for source in &sources.sources {
            for table in &source.tables {
                source_entries.push(ListEntry {
                    address: format!("smelt.sources.{}.{}", source.name, table.name),
                    kind: EntityKind::Source,
                    materialization: None,
                });
            }
        }
    }

    let mut entries: Vec<ListEntry> = Vec::new();
    entries.append(&mut model_entries);
    entries.append(&mut seed_entries);
    entries.append(&mut source_entries);
    entries.append(&mut test_entries);
    entries.append(&mut check_entries);
    entries.sort_by(|a, b| a.address.cmp(&b.address));

    match args.format.as_str() {
        "json" => {
            let output = serde_json::to_string_pretty(&entries)
                .expect("ListEntry serialization is infallible");
            println!("{output}");
        }
        _ => {
            for entry in &entries {
                match &entry.materialization {
                    Some(m) => println!("{}  {}  {}", entry.address, entry.kind, m),
                    None => println!("{}  {}", entry.address, entry.kind),
                }
            }
        }
    }

    Ok(())
}
