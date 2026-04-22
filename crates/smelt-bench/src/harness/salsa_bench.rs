use crate::model_gen::GeneratedWorkspace;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

/// Metrics from Salsa incremental compilation benchmarks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalsaMetrics {
    /// Time to load all files into Salsa and warm caches (ms).
    pub initial_load_ms: f64,
    /// Time to recompute diagnostics after editing a Layer 1 model (ms).
    pub leaf_edit_diagnostics_ms: f64,
    /// Time to recompute diagnostics after editing a Layer 2 model (ms).
    pub mid_edit_diagnostics_ms: f64,
    /// Time to recompute diagnostics after editing a Layer 4 model (ms).
    pub root_edit_diagnostics_ms: f64,
    /// Time to recompute all_models() after adding a new file (ms).
    pub add_file_all_models_ms: f64,
    /// Time to compute file_diagnostics for ALL models (ms).
    pub full_diagnostics_ms: f64,
}

/// Run Salsa/LSP edit benchmarks on a generated workspace.
///
/// Only uses SQL models (Python models produce SQL which is what Salsa sees).
pub fn run_salsa_benchmark(workspace: &GeneratedWorkspace) -> Result<SalsaMetrics> {
    // Collect SQL model file paths and contents
    let sql_files: Vec<(PathBuf, String)> = workspace
        .sql_contents
        .iter()
        .map(|(name, content)| {
            let path = workspace.models_path().join(format!("{}.sql", name));
            (path, content.clone())
        })
        .collect();

    if sql_files.is_empty() {
        return Ok(SalsaMetrics {
            initial_load_ms: 0.0,
            leaf_edit_diagnostics_ms: 0.0,
            mid_edit_diagnostics_ms: 0.0,
            root_edit_diagnostics_ms: 0.0,
            add_file_all_models_ms: 0.0,
            full_diagnostics_ms: 0.0,
        });
    }

    // --- Phase 1: Initial load and warm ---
    let load_start = Instant::now();

    let mut db = smelt_db::Database::default();
    let project_root = workspace.path().to_path_buf();

    // Register all source files
    let mut source_files = Vec::with_capacity(sql_files.len());
    for (path, content) in &sql_files {
        let sf = db.set_source_file(path.clone(), content.clone(), project_root.clone());
        source_files.push(sf);
    }

    // Set sources YAML
    let sources_yml =
        std::fs::read_to_string(workspace.path().join("sources.yml")).unwrap_or_default();
    let project = db.set_project_input(project_root.clone(), sources_yml);
    db.set_workspace(source_files.clone(), vec![project]);

    // Warm caches
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let _models = smelt_db::all_models(&db, ws);
    for (path, _) in &sql_files {
        if let Some(file) = db.source_file(path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
    }

    let initial_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 2: Leaf edit (Layer 1 model) ---
    let leaf_path = sql_files
        .iter()
        .find(|(p, _)| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.contains("_l1_"))
        })
        .map(|(p, _)| p.clone());

    let leaf_edit_diagnostics_ms = if let Some(path) = leaf_path {
        let edit_start = Instant::now();
        db.set_source_file(
            path.clone(),
            "SELECT 1 AS edited_column\n".to_string(),
            project_root.clone(),
        );
        let ws = smelt_db::Workspace::try_get(&db).unwrap();
        if let Some(file) = db.source_file(&path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
        edit_start.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };

    // --- Phase 3: Mid edit (Layer 2 model) ---
    let mid_path = sql_files
        .iter()
        .find(|(p, _)| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.contains("_l2_"))
        })
        .map(|(p, _)| p.clone());

    let mid_edit_diagnostics_ms = if let Some(path) = mid_path {
        let edit_start = Instant::now();
        db.set_source_file(
            path.clone(),
            "SELECT 1 AS edited_mid_column\n".to_string(),
            project_root.clone(),
        );
        let ws = smelt_db::Workspace::try_get(&db).unwrap();
        if let Some(file) = db.source_file(&path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
        edit_start.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };

    // --- Phase 4: Root edit (Layer 4 model) ---
    let root_path = sql_files
        .iter()
        .find(|(p, _)| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.contains("_l4_"))
        })
        .map(|(p, _)| p.clone());

    let root_edit_diagnostics_ms = if let Some(path) = root_path {
        let edit_start = Instant::now();
        db.set_source_file(
            path.clone(),
            "SELECT 1 AS edited_root_column\n".to_string(),
            project_root.clone(),
        );
        let ws = smelt_db::Workspace::try_get(&db).unwrap();
        if let Some(file) = db.source_file(&path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
        edit_start.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };

    // --- Phase 5: Add new file ---
    let add_start = Instant::now();
    let new_path = workspace.models_path().join("new_model_bench.sql");
    let new_sf = db.set_source_file(new_path, "SELECT 1 AS new_col\n".to_string(), project_root);
    let mut updated_files = source_files;
    updated_files.push(new_sf);
    let project = db.project_input(workspace.path()).unwrap();
    db.set_workspace(updated_files, vec![project]);
    let ws = smelt_db::Workspace::try_get(&db).unwrap();
    let _models = smelt_db::all_models(&db, ws);
    let add_file_all_models_ms = add_start.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 6: Full diagnostics ---
    let full_start = Instant::now();
    let ws = smelt_db::Workspace::try_get(&db).unwrap();
    for (path, _) in &sql_files {
        if let Some(file) = db.source_file(path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
    }
    let full_diagnostics_ms = full_start.elapsed().as_secs_f64() * 1000.0;

    Ok(SalsaMetrics {
        initial_load_ms,
        leaf_edit_diagnostics_ms,
        mid_edit_diagnostics_ms,
        root_edit_diagnostics_ms,
        add_file_all_models_ms,
        full_diagnostics_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gen::{generate_workspace, GraphSpec};

    #[test]
    fn test_salsa_benchmark_small() {
        let spec = GraphSpec::small();
        let workspace = generate_workspace(&spec).unwrap();
        let metrics = run_salsa_benchmark(&workspace).unwrap();

        assert!(metrics.initial_load_ms >= 0.0);
        assert!(metrics.full_diagnostics_ms >= 0.0);
    }
}
