use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use notify_debouncer_mini::{new_debouncer, notify, DebouncedEventKind};
use smelt_core::discovery::ModelDiscovery;
use smelt_core::graph::DependencyGraph;
use smelt_db::SourceFile;

use crate::server::{AppState, ChangeEvent};

/// Start a file watcher that monitors model directories and sources.yml.
///
/// On changes, re-reads files from disk, updates the Salsa database,
/// rebuilds the dependency graph, and notifies WebSocket clients.
pub fn start_watcher(
    state: Arc<AppState>,
    model_dirs: Vec<PathBuf>,
    project_dir: PathBuf,
) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut debouncer = new_debouncer(std::time::Duration::from_millis(200), tx)
        .with_context(|| "Failed to create file watcher")?;

    // Watch model directories
    for dir in &model_dirs {
        if dir.exists() {
            debouncer
                .watcher()
                .watch(dir, notify::RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch directory: {:?}", dir))?;
        }
    }

    // Watch sources.yml if it exists
    for name in &["sources.yml", "sources.yaml"] {
        let path = project_dir.join(name);
        if path.exists() {
            debouncer
                .watcher()
                .watch(&path, notify::RecursiveMode::NonRecursive)
                .with_context(|| format!("Failed to watch {:?}", path))?;
        }
    }

    // Watch smelt.yml
    for name in &["smelt.yml", "smelt.yaml"] {
        let path = project_dir.join(name);
        if path.exists() {
            debouncer
                .watcher()
                .watch(&path, notify::RecursiveMode::NonRecursive)
                .with_context(|| format!("Failed to watch {:?}", path))?;
        }
    }

    let paths_config = state.config.paths.clone();

    // Spawn a background thread to process file events
    // (notify uses std channels, not tokio)
    std::thread::spawn(move || {
        // Keep debouncer alive
        let _debouncer = debouncer;

        while let Ok(result) = rx.recv() {
            match result {
                Ok(events) => {
                    let has_sql_changes = events
                        .iter()
                        .any(|e| e.kind == DebouncedEventKind::Any && is_relevant_file(&e.path));

                    if has_sql_changes {
                        tracing::info!("File change detected, refreshing...");
                        let state = state.clone();
                        let project_dir = project_dir.clone();
                        let paths = paths_config.clone();

                        // Use a tokio runtime handle to do async work
                        tokio::task::block_in_place(move || {
                            tokio::runtime::Handle::current().block_on(async {
                                if let Err(e) = refresh_state(&state, &project_dir, &paths).await {
                                    tracing::error!("Failed to refresh state: {}", e);
                                }
                            });
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("File watcher error: {:?}", e);
                }
            }
        }
    });

    Ok(())
}

fn is_relevant_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(ext, "sql" | "yml" | "yaml")
}

async fn refresh_state(state: &AppState, project_dir: &Path, paths: &[String]) -> Result<()> {
    // Re-discover models from disk
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), paths.to_vec());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to rediscover models")?;

    // Update Salsa database
    {
        let mut db = state.db.lock().await;
        let mut source_files: Vec<SourceFile> = Vec::with_capacity(models.len());
        for model in &models {
            let content = std::fs::read_to_string(&model.path).unwrap_or_default();
            let sf = db.set_source_file(model.path.clone(), content, project_dir.to_path_buf());
            source_files.push(sf);
        }

        // Re-read sources.yml
        let sources_yaml = smelt_core::find_config_file(project_dir, "sources")
            .ok()
            .flatten()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let project = db.set_project_input(project_dir.to_path_buf(), sources_yaml);
        db.set_workspace(source_files, vec![project]);
    }

    // Rebuild dependency graph. `models` is `Vec<smelt_core::ModelFile>` so
    // the previous field-by-field rebuild is redundant since the type was
    // unified; just clone the slice.
    let sources = smelt_core::SourcesConfig::load(project_dir).ok();
    let core_models: Vec<smelt_core::ModelFile> = models.iter().cloned().collect();

    if let Ok(new_graph) = DependencyGraph::build(core_models, sources.as_ref()) {
        let mut graph = state.graph.lock().await;
        *graph = new_graph;
    }

    // Notify WebSocket clients
    let _ = state.change_tx.send(ChangeEvent::ModelsUpdated);

    tracing::info!("State refreshed: {} models", models.len());
    Ok(())
}
