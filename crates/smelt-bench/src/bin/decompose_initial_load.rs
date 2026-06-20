//! Privilege-free decomposition of the cold "initial load" phase.
//!
//! Splits the warm-up into its constituent Salsa passes, timing each against a
//! FRESH database so we see where the cold cost concentrates:
//!   1. ingest        — set_source_file + set_workspace (input creation)
//!   2. all_models    — workspace model graph
//!   3. type_context  — warm type_context() for every file (schema/type infer)
//!   4. diagnostics   — file_diagnostics() for every file (full check, incl. above)
//!
//! Passes 3 and 4 are each measured from a fresh DB so they include everything
//! they transitively force; (4 - 3) approximates the non-type-inference
//! diagnostic work, and 3 itself is dominated by ref/schema resolution.

use anyhow::Result;
use smelt_bench::model_gen::{generate_workspace, GraphSpec};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn build_db(
    sql_files: &[(PathBuf, String)],
    project_root: &Path,
    sources_yml: &str,
) -> (smelt_db::Database, smelt_db::Workspace) {
    let mut db = smelt_db::Database::default();
    let mut source_files = Vec::with_capacity(sql_files.len());
    for (path, content) in sql_files {
        source_files.push(db.set_source_file(
            path.clone(),
            content.clone(),
            project_root.to_path_buf(),
        ));
    }
    let project = db.set_project_input(project_root.to_path_buf(), sources_yml.to_string());
    db.set_workspace(source_files, vec![project]);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace");
    (db, ws)
}

fn main() -> Result<()> {
    let reps: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    eprintln!("Generating benchmark workspace (2000 models)...");
    let workspace = generate_workspace(&GraphSpec::default())?;
    let sql_files: Vec<(PathBuf, String)> = workspace
        .sql_contents
        .iter()
        .map(|(name, content)| {
            (
                workspace.models_path().join(format!("{}.sql", name)),
                content.clone(),
            )
        })
        .collect();
    let project_root = workspace.path().to_path_buf();
    let sources_yml =
        std::fs::read_to_string(workspace.path().join("sources.yml")).unwrap_or_default();
    eprintln!(
        "Decomposing initial load over {} files, best of {reps}\n",
        sql_files.len()
    );

    let mut best_ingest = f64::MAX;
    let mut best_all_models = f64::MAX;
    let mut best_type_ctx = f64::MAX;
    let mut best_diags = f64::MAX;

    for _ in 0..reps {
        // 1. ingest
        let t = Instant::now();
        let (db, ws) = build_db(&sql_files, &project_root, &sources_yml);
        best_ingest = best_ingest.min(t.elapsed().as_secs_f64() * 1000.0);

        // 2. all_models on that same fresh db
        let t = Instant::now();
        let _ = smelt_db::all_models(&db, ws);
        best_all_models = best_all_models.min(t.elapsed().as_secs_f64() * 1000.0);
        drop(db);

        // 3. type_context for every file, fresh db
        let (db, ws) = build_db(&sql_files, &project_root, &sources_yml);
        let _ = smelt_db::all_models(&db, ws);
        let t = Instant::now();
        for (path, _) in &sql_files {
            if let Some(file) = db.source_file(path) {
                let _ = smelt_db::type_context(&db, ws, file);
            }
        }
        best_type_ctx = best_type_ctx.min(t.elapsed().as_secs_f64() * 1000.0);
        drop(db);

        // 4. file_diagnostics for every file, fresh db
        let (db, ws) = build_db(&sql_files, &project_root, &sources_yml);
        let _ = smelt_db::all_models(&db, ws);
        let t = Instant::now();
        for (path, _) in &sql_files {
            if let Some(file) = db.source_file(path) {
                let _ = smelt_db::file_diagnostics(&db, ws, file);
            }
        }
        best_diags = best_diags.min(t.elapsed().as_secs_f64() * 1000.0);
        drop(db);
    }

    eprintln!("--- best-of-{reps} (ms), each from a fresh DB ---");
    eprintln!("1. ingest (inputs)        {:>8.1}", best_ingest);
    eprintln!("2. all_models             {:>8.1}", best_all_models);
    eprintln!("3. type_context (all)     {:>8.1}", best_type_ctx);
    eprintln!("4. file_diagnostics (all) {:>8.1}", best_diags);
    eprintln!(
        "   └ diag work beyond TC  {:>8.1}  (4 - 3)",
        best_diags - best_type_ctx
    );
    Ok(())
}
