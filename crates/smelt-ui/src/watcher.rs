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

    // Capture a handle to the current Tokio runtime *now*, while we are still
    // running inside it (`start_watcher` is called from the async server
    // startup). The background thread below is a plain std thread with no
    // runtime context of its own, so `Handle::current()` there would panic
    // ("there is no reactor running"). The captured handle lets that thread
    // drive async work via `handle.block_on(...)`.
    let runtime = tokio::runtime::Handle::current();

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

                        // Drive the async refresh on the captured runtime. This
                        // thread is not a runtime worker, so we call `block_on`
                        // on the handle directly (not `block_in_place`, which
                        // must run from within a runtime worker thread).
                        runtime.block_on(async {
                            if let Err(e) = refresh_state(&state, &project_dir, &paths).await {
                                tracing::error!("Failed to refresh state: {}", e);
                            }
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
    matches!(ext, "sql" | "yml" | "yaml" | "py")
}

async fn refresh_state(state: &AppState, project_dir: &Path, paths: &[String]) -> Result<()> {
    // Re-discover models from disk
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), paths.to_vec());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to rediscover models")?;

    // Discover Python models via the shared runtime entry point (Run Pipeline
    // Parity rule — the UI now includes Python models just like the CLI).
    let mut all_models = sql_models.clone();
    if let Ok(python_files) = discovery.discover_python_files() {
        if !python_files.is_empty() {
            match smelt_runtime::discover_python_models(
                &python_files,
                &sql_models,
                &state.config,
                project_dir,
                state.config.python.as_deref(),
            ) {
                Ok(python_models) => {
                    if !python_models.is_empty() {
                        tracing::info!(
                            "File watcher: discovered {} Python model(s)",
                            python_models.len()
                        );
                        all_models.extend(python_models);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to discover Python models on file change: {}", e);
                }
            }
        }
    }

    // Update Salsa database
    {
        let mut db = state.db.lock().await;
        let mut source_files: Vec<SourceFile> = Vec::with_capacity(all_models.len());
        for model in &all_models {
            // Use model.content directly: SQL models carry the file text read
            // during discovery; Python models carry the normalized SQL produced
            // by discover_python_models. Reading the .py path from disk here
            // would store raw Python syntax where Salsa expects generated SQL.
            let sf = db.set_source_file(
                model.path.clone(),
                model.content.clone(),
                project_dir.to_path_buf(),
            );
            source_files.push(sf);
        }
        // Register function files so smelt.functions.* calls resolve.
        for fn_path in smelt_core::discover_function_file_paths(project_dir) {
            let content = std::fs::read_to_string(&fn_path).unwrap_or_default();
            let sf = db.set_source_file(fn_path, content, project_dir.to_path_buf());
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

    // Rebuild dependency graph including Python models.
    let sources = smelt_core::SourcesConfig::load(project_dir).ok();
    let model_count = all_models.len();

    if let Ok(new_graph) = DependencyGraph::build(all_models, sources.as_ref()) {
        let mut graph = state.graph.lock().await;
        *graph = new_graph;
    }

    // Notify WebSocket clients
    let _ = state.change_tx.send(ChangeEvent::ModelsUpdated);

    tracing::info!("State refreshed: {} models", model_count);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::AppState;
    use smelt_core::config::Config;
    use smelt_core::graph::DependencyGraph;
    use std::collections::HashMap;
    use tokio::sync::broadcast;

    fn minimal_state(project_dir: &Path) -> Arc<AppState> {
        let config = Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: HashMap::new(),
            default_materialization: smelt_core::config::Materialization::View,
            models: HashMap::new(),
            python: None,
            target: None,
            state: Default::default(),
            maintenance: None,
        };

        let (change_tx, _) = broadcast::channel(16);
        let (run_event_tx, _) = broadcast::channel(16);
        let run_manager = Arc::new(crate::run_manager::RunManager::new(
            run_event_tx,
            project_dir.to_path_buf(),
        ));

        Arc::new(AppState {
            db: Arc::new(tokio::sync::Mutex::new(smelt_db::Database::default())),
            config: Arc::new(config),
            sources: Arc::new(None),
            graph: Arc::new(tokio::sync::Mutex::new(
                DependencyGraph::build(vec![], None).unwrap(),
            )),
            project_dir: project_dir.to_path_buf(),
            change_tx,
            run_manager,
            run_event_tx: broadcast::channel(16).0,
        })
    }

    /// A file change in a watched directory must drive `refresh_state` to
    /// completion and broadcast a `ModelsUpdated` event. Regression test for
    /// the watcher thread panicking with "there is no reactor running" because
    /// it called `tokio::runtime::Handle::current()` from a plain std thread
    /// that is not inside the Tokio runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_change_broadcasts_models_updated() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().to_path_buf();
        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let model_file = models_dir.join("foo.sql");
        std::fs::write(&model_file, "SELECT 1 AS id").unwrap();

        let state = minimal_state(&project_dir);
        let mut change_rx = state.change_tx.subscribe();

        start_watcher(state, vec![models_dir.clone()], project_dir).unwrap();

        // Give the watcher a moment to register, then modify the file.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::fs::write(&model_file, "SELECT 2 AS id").unwrap();

        // The watcher thread must process the event without panicking and
        // broadcast ModelsUpdated. Before the fix it panicked, so this times out.
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), change_rx.recv())
            .await
            .expect("timed out waiting for ModelsUpdated — watcher thread likely panicked")
            .expect("change channel closed");

        assert!(matches!(event, ChangeEvent::ModelsUpdated));
    }
}
