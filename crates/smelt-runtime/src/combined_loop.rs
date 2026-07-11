//! Combined fixed-point discovery loop for SQL generators + Python @model files.
//!
//! Implements the interleaved evaluation loop from the python_models spec
//! §"Iterative evaluation" and meta_language.md §"Multi-model production" rule 4.
//! Both generator families run per round; the loop stops when the full model set
//! (keyed by canonical address + frontmatter + SQL content) is byte-identical to
//! the prior round. Within a round, generators are evaluated in ascending path order
//! (python_models spec §"Determinism and termination").
//!
//! Each round, the Salsa DB is rebuilt with `base_sql_models + prev_python` so
//! that SQL generators can resolve literal refs to Python-emitted models from the
//! prior round (inter-round visibility). Python receives the same accumulated set
//! so Python models can reference prior-round SQL generator output. Intra-round
//! `smelt.models.*` reflection remains forbidden (enforced by AST check in
//! `smelt-db`'s `check_generator_body_reflection_forbid`).

use anyhow::Result;
use smelt_core::config::{Config, Materialization};
use smelt_core::discovery::{discover_function_file_paths, parse_sql_file, ModelFile, ModelKind};
use smelt_core::find_config_file;
use smelt_core::metadata::ModelMetadata;
use smelt_core::ModelId;
use smelt_db::EmittedModelDef;
use smelt_parser::File as AstFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Run the combined SQL-generator + Python fixed-point discovery loop.
///
/// Each round: (1) rebuild the Salsa DB with `base_sql_models + prev_python`;
/// (2) run the SQL-generator pass so generators can resolve literal refs against
/// prior-round Python emissions; (3) run Python discovery against the full
/// accumulated set (`non_gen_sql + emitted_sql + prev_python`); (4) compare the
/// resulting model set (address, frontmatter, SQL content — keyed by name) to the
/// prior round. Loop terminates when byte-identical; errors after MAX_ROUNDS if
/// unstable.
///
/// Within a round, Python files are evaluated in ascending path order
/// (spec §"Determinism and termination"). SQL generators are similarly ordered
/// by the Salsa `emitted_models` pass.
///
/// The `active_target` parameter overrides the target used when evaluating SQL
/// generators (controls which overlay files are loaded). When `None`, the Salsa
/// DB falls back to the `target:` field in `smelt.yml`. The CLI passes the
/// `--target` flag value here; the UI typically leaves it `None` (relying on
/// the smelt.yml default).
pub fn run_combined_discovery_loop(
    base_sql_models: Vec<ModelFile>,
    python_files: Vec<(PathBuf, Vec<u32>, String)>,
    project_dir: &Path,
    config: &Config,
    config_python: Option<&str>,
    active_target: Option<&str>,
) -> Result<Vec<ModelFile>> {
    const MAX_ROUNDS: usize = 5;

    // Non-generator SQL models: exclude .gen. files and models with generates: frontmatter.
    let non_gen_sql: Vec<ModelFile> = base_sql_models
        .iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .filter(|m| m.metadata.as_ref().is_none_or(|md| md.generates.is_none()))
        .cloned()
        .collect();

    if python_files.is_empty() {
        // Fast path: no Python. Run the SQL generator pass once against base models only.
        let db = build_generator_db(project_dir, &base_sql_models, active_target);
        let emitted_sql = run_sql_generator_pass(&db, project_dir, &config.paths);
        if !emitted_sql.is_empty() {
            debug!(
                "Combined loop (no Python): SQL generator pass produced {} emitted model(s)",
                emitted_sql.len()
            );
        }
        let mut result = non_gen_sql;
        result.extend(emitted_sql);
        return Ok(result);
    }

    // Sort python_files by path for deterministic within-round evaluation order
    // (spec §"Determinism and termination": ascending by path, then name as tiebreaker).
    let mut sorted_python_files = python_files;
    sorted_python_files.sort_by(|(a, ..), (b, ..)| a.to_string_lossy().cmp(&b.to_string_lossy()));

    // Combined fixed-point loop.
    // prev_python: Python-emitted models from the prior round, fed to the Salsa DB
    // each round so SQL generators can resolve literal refs against them.
    let mut prev_python: Vec<ModelFile> = Vec::new();
    let mut prev_set_key: Vec<(String, String, String)> = Vec::new();

    for round in 0..MAX_ROUNDS {
        debug!("Combined loop: round {}", round + 1);

        // Rebuild Salsa DB each round: seed with base SQL models + prior-round Python
        // so generators can resolve literal refs to Python-emitted names.
        //
        // Python models from the same .py file share a physical path; build_generator_db
        // deduplicates by path, so we assign each Python model a unique virtual path
        // (same pattern as emitted SQL models in model_file_from_emitted_def).
        let db_models: Vec<ModelFile> = base_sql_models
            .iter()
            .cloned()
            .chain(prev_python.iter().map(|m| {
                let mut m = m.clone();
                m.path = m.path.with_file_name(format!(
                    "{}::{}",
                    m.path.file_name().and_then(|n| n.to_str()).unwrap_or("py"),
                    m.name
                ));
                m
            }))
            .collect();
        let db = build_generator_db(project_dir, &db_models, active_target);
        let emitted_sql = run_sql_generator_pass(&db, project_dir, &config.paths);

        if !emitted_sql.is_empty() {
            debug!(
                "Combined loop: round {} SQL generator pass produced {} emitted model(s)",
                round + 1,
                emitted_sql.len()
            );
        }

        // Context for Python discovery: non-gen SQL + emitted SQL + prior-round Python.
        // This lets Python models reference SQL generator output AND each other's prior
        // emissions; discover_python_models handles its own intra-round inner loop.
        let sql_context: Vec<ModelFile> = non_gen_sql
            .iter()
            .chain(emitted_sql.iter())
            .chain(prev_python.iter())
            .cloned()
            .collect();

        // Run Python discovery (discover_python_models handles its own inner loop).
        let new_python = crate::python::discover_python_models(
            &sorted_python_files,
            &sql_context,
            config,
            project_dir,
            config_python,
        )?;

        debug!(
            "Combined loop: round {} produced {} Python model(s)",
            round + 1,
            new_python.len()
        );

        // Compute a byte-equal key for the full model set of this round.
        // The spec requires byte-equality of canonical address, frontmatter, AND SQL content
        // (python_models.md §"Determinism and termination"). The key covers non_gen_sql +
        // emitted_sql + new_python (not prev_python — that was the prior round's output).
        let mut new_key: Vec<(String, String, String)> = non_gen_sql
            .iter()
            .chain(emitted_sql.iter())
            .chain(new_python.iter())
            .map(|m| {
                let meta_key = format!("{:?}", m.metadata.as_deref());
                (m.name.clone(), m.content.clone(), meta_key)
            })
            .collect();
        new_key.sort();

        if new_key == prev_set_key {
            debug!("Combined loop: converged after {} round(s)", round + 1);
            // Sort the final result by (path, name) for deterministic output order
            // (spec §"Determinism and termination": path-then-name within a round).
            let mut result: Vec<ModelFile> = non_gen_sql
                .iter()
                .chain(emitted_sql.iter())
                .chain(new_python.iter())
                .cloned()
                .collect();
            result.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
            return Ok(result);
        }

        prev_set_key = new_key;
        prev_python = new_python;
    }

    Err(anyhow::anyhow!(
        "Combined discovery loop did not converge after {} rounds. \
         This likely indicates a circular dependency in Python model generation.",
        MAX_ROUNDS
    ))
}

/// Build a Salsa database seeded with the given SQL model files.
///
/// Replicates the logic of `smelt_cli::init_db` but lives in smelt-runtime
/// so that `smelt-runtime` has no dependency on `smelt-cli`. The CLI and UI
/// both call `run_combined_discovery_loop` which uses this internally.
///
/// `active_target` — when `Some`, sets the active build target on the database
/// (controls overlay file selection for `smelt.config.load_yaml` calls).
/// When `None`, the target falls back to the `target:` field from `smelt.yml`.
///
/// Invariant: Workspace Loading Parity rule (CLI ↔ LSP) is respected — loader
/// file registration goes through
/// `smelt_db::workspace_ingest::register_loader_files_from_disk`.
fn build_generator_db(
    project_dir: &Path,
    models: &[ModelFile],
    active_target: Option<&str>,
) -> smelt_db::Database {
    let mut db = smelt_db::Database::default();

    // Load sources.yml or sources.yaml.
    let sources_yaml = match find_config_file(project_dir, "sources") {
        Ok(Some(path)) => std::fs::read_to_string(&path).unwrap_or_default(),
        Ok(None) => String::new(),
        Err(msg) => {
            tracing::warn!("{}", msg);
            String::new()
        }
    };
    let project = db.set_project_input(project_dir.to_path_buf(), sources_yaml);

    // Discover function files for the project using smelt-core's path scanner.
    // This mirrors what smelt_cli::init_db does (Workspace Loading Parity rule).
    let mut function_files: Vec<ModelFile> = Vec::new();
    for path in discover_function_file_paths(project_dir) {
        match parse_sql_file(&path, Some(project_dir)) {
            Ok(parsed) => function_files.extend(parsed),
            Err(err) => tracing::warn!("Failed to parse function file {}: {}", path.display(), err),
        }
    }

    // Register model files + function files, deduplicating by path.
    let mut source_files = Vec::with_capacity(models.len() + function_files.len());
    let mut seen_paths: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for model in models.iter().chain(function_files.iter()) {
        if !seen_paths.insert(model.path.clone()) {
            continue;
        }
        let sf = db.set_source_file(
            model.path.clone(),
            model.content.clone(),
            project_dir.to_path_buf(),
        );
        source_files.push(sf);
    }
    db.set_workspace(source_files, vec![project]);

    // Register YAML/JSON/TOML loader files (for smelt.config.load_yaml / load_json).
    smelt_db::workspace_ingest::register_loader_files_from_disk(&mut db, project_dir);

    // Set the active target: prefer the caller-supplied override (e.g. from --target),
    // then fall back to the `target:` field from smelt.yml.
    let resolved_target: Option<std::sync::Arc<str>> = if let Some(t) = active_target {
        Some(std::sync::Arc::from(t))
    } else if let Ok(cfg) = smelt_core::config::Config::load(project_dir) {
        cfg.target.map(|t| std::sync::Arc::from(t.as_str()))
    } else {
        None
    };
    db.set_active_target(resolved_target);

    db
}

/// Run the Salsa emitted-models pass and return the virtual `ModelFile`s.
///
/// Replicates `smelt_cli::discover_emitted_model_files` (without the origins
/// map) so that `smelt-runtime` can call it without depending on `smelt-cli`.
fn run_sql_generator_pass(
    db: &smelt_db::Database,
    project_dir: &Path,
    scan_roots: &[String],
) -> Vec<ModelFile> {
    let workspace = match smelt_db::Workspace::try_get(db) {
        Some(ws) => ws,
        None => return Vec::new(),
    };

    let result = smelt_db::emitted_models(db, workspace);
    let mut model_files = Vec::new();

    for emitted in &result.survivors {
        let smelt_name = smelt_db::emitted_model_smelt_path(
            &emitted.generator_file,
            project_dir,
            scan_roots,
            &emitted.name,
        );
        let mf = model_file_from_emitted_def(emitted, smelt_name);
        model_files.push(mf);
    }

    model_files
}

/// Build a virtual `ModelFile` from an `EmittedModelDef`.
///
/// Duplicates `smelt_cli::discovery::model_file_from_emitted_def` — placed
/// here to keep the dependency direction clean (`smelt-runtime` must not
/// import `smelt-cli`).
fn model_file_from_emitted_def(emitted: &EmittedModelDef, smelt_name: String) -> ModelFile {
    use smelt_core::extract_refs;

    // Parse the body text for ref extraction.
    let content = emitted.body_text.clone();
    let parse = smelt_parser::parse(&content);
    let refs = if let Some(file) = AstFile::cast(parse.syntax()) {
        extract_refs(&file)
    } else {
        Vec::new()
    };

    // Build minimal metadata from emitted fields.
    let materialization = match emitted.materialization.as_str() {
        "table" | "incremental" => Some(Materialization::Table),
        _ => Some(Materialization::View),
    };
    let metadata = Box::new(ModelMetadata {
        name: Some(smelt_name.clone()),
        generates: None,
        materialization,
        timeseries: emitted.timeseries_config.clone(),
        refresh: if emitted.incremental_config.is_some() {
            Some(smelt_core::config::RefreshStrategy::Incremental)
        } else {
            None
        },
        grain: if emitted.incremental_config.is_some() {
            Some(smelt_core::config::Grain::Partition)
        } else {
            None
        },
        batched: emitted.incremental_config.clone(),
        target: None,
        tags: emitted.tags.clone(),
        owner: None,
        description: if emitted.description.is_empty() {
            None
        } else {
            Some(emitted.description.clone())
        },
        columns: std::collections::HashMap::new(),
        backend_hints: std::collections::HashMap::new(),
        test: None,
        check: None,
        schema_evolution: None,
        format: None,
        reuse: None,
        forward_only: false,
        state: None,
        functional_dependencies: Vec::new(),
        bounded_domain: None,
        horizon_ceiling: None,
        maintenance: None,
    });

    // Virtual path: generator_file path with model name appended as a virtual
    // component so the Salsa key is unique per emission.
    let virtual_path = emitted.generator_file.with_file_name(format!(
        "{}::{}",
        emitted
            .generator_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("gen"),
        smelt_name
    ));
    let model_id = ModelId::multi_model(emitted.generator_file.clone(), smelt_name.clone());

    // Address segments: smelt_name split by '.'.
    let address_segments: Vec<String> = smelt_name.split('.').map(|s| s.to_string()).collect();

    ModelFile {
        name: smelt_name,
        path: virtual_path,
        content,
        refs,
        parse_errors: parse.errors,
        metadata: Some(metadata),
        kind: ModelKind::Sql,
        model_id,
        address_segments,
    }
}
