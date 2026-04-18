use criterion::{criterion_group, criterion_main, Criterion};
use smelt_bench::model_gen::{generate_workspace, GraphSpec};
use std::path::PathBuf;

fn setup_salsa_db(
    workspace: &smelt_bench::model_gen::GeneratedWorkspace,
) -> (smelt_db::Database, Vec<PathBuf>) {
    let mut db = smelt_db::Database::default();
    let project_root = workspace.path().to_path_buf();
    let mut source_files = Vec::new();
    let mut all_paths = Vec::new();

    for (name, content) in &workspace.sql_contents {
        let path = workspace.models_path().join(format!("{}.sql", name));
        let sf = db.set_source_file(path.clone(), content.clone(), project_root.clone());
        source_files.push(sf);
        all_paths.push(path);
    }

    let sources_yml =
        std::fs::read_to_string(workspace.path().join("sources.yml")).unwrap_or_default();
    let project = db.set_project_input(project_root, sources_yml);
    db.set_workspace(source_files, vec![project]);

    (db, all_paths)
}

fn bench_initial_load(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    c.bench_function("salsa_initial_load_2000", |b| {
        b.iter(|| {
            let (db, _) = setup_salsa_db(&workspace);
            let ws = smelt_db::Workspace::try_get(&db).unwrap();
            let _models = smelt_db::all_models(&db, ws);
        })
    });
}

fn bench_leaf_edit(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");
    let (mut db, all_paths) = setup_salsa_db(&workspace);
    let project_root = workspace.path().to_path_buf();

    // Warm caches
    let ws = smelt_db::Workspace::try_get(&db).unwrap();
    let _models = smelt_db::all_models(&db, ws);
    for path in &all_paths {
        if let Some(file) = db.source_file(path) {
            let _diags = smelt_db::file_diagnostics(&db, ws, file);
        }
    }

    // Find a layer 1 model
    let leaf_path = all_paths
        .iter()
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.contains("_l1_"))
        })
        .cloned()
        .unwrap_or_else(|| all_paths[0].clone());

    c.bench_function("salsa_leaf_edit_diagnostics", |b| {
        b.iter(|| {
            db.set_source_file(
                leaf_path.clone(),
                "SELECT 1 AS bench_col\n".to_string(),
                project_root.clone(),
            );
            let ws = smelt_db::Workspace::try_get(&db).unwrap();
            if let Some(file) = db.source_file(&leaf_path) {
                let _diags = smelt_db::file_diagnostics(&db, ws, file);
            }
        })
    });
}

fn bench_full_diagnostics(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");
    let (db, all_paths) = setup_salsa_db(&workspace);

    // Warm caches
    let ws = smelt_db::Workspace::try_get(&db).unwrap();
    let _models = smelt_db::all_models(&db, ws);

    c.bench_function("salsa_full_diagnostics_2000", |b| {
        b.iter(|| {
            let ws = smelt_db::Workspace::try_get(&db).unwrap();
            for path in &all_paths {
                if let Some(file) = db.source_file(path) {
                    let _diags = smelt_db::file_diagnostics(&db, ws, file);
                }
            }
        })
    });
}

criterion_group!(
    benches,
    bench_initial_load,
    bench_leaf_edit,
    bench_full_diagnostics
);
criterion_main!(benches);
