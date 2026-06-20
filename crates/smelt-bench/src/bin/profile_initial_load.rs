//! Focused profiling target: run ONLY the cold "initial load" workload in a
//! loop so a flamegraph/perf profile is dominated by it (not by workspace
//! generation or the edit phases).
//!
//! Each iteration builds a fresh `Database`, registers every SQL file, then
//! warms `all_models` + `file_diagnostics` for all files — exactly the
//! `initial_load_ms` phase of `salsa_bench.rs`.
//!
//! Usage:
//!   cargo run -p smelt-bench --bin profile_initial_load --release -- [iters]
//! Under perf/flamegraph:
//!   flamegraph -o il.svg -- target/release/profile_initial_load 40

use anyhow::Result;
use smelt_bench::model_gen::{generate_workspace, GraphSpec};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let iters: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);

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
        "Profiling initial load: {iters} iterations over {} files",
        sql_files.len()
    );

    let mut best = f64::MAX;
    let mut total = 0.0;
    for i in 0..iters {
        let start = Instant::now();

        let mut db = smelt_db::Database::default();
        let mut source_files = Vec::with_capacity(sql_files.len());
        for (path, content) in &sql_files {
            source_files.push(db.set_source_file(
                path.clone(),
                content.clone(),
                project_root.clone(),
            ));
        }
        let project = db.set_project_input(project_root.clone(), sources_yml.clone());
        db.set_workspace(source_files, vec![project]);

        let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
        let _models = smelt_db::all_models(&db, ws);
        for (path, _) in &sql_files {
            if let Some(file) = db.source_file(path) {
                let _diags = smelt_db::file_diagnostics(&db, ws, file);
            }
        }

        let ms = start.elapsed().as_secs_f64() * 1000.0;
        best = best.min(ms);
        total += ms;
        if i == 0 || (i + 1) % 10 == 0 {
            eprintln!("  iter {:>3}: {:.1} ms", i + 1, ms);
        }
    }

    eprintln!(
        "initial_load: best {:.1} ms, mean {:.1} ms ({iters} iters)",
        best,
        total / iters as f64
    );
    Ok(())
}
