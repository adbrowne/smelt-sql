use criterion::{criterion_group, criterion_main, Criterion};
use smelt_bench::model_gen::{generate_workspace, GraphSpec};
use smelt_db::{Inputs, Semantic, Syntax};
use std::path::PathBuf;
use std::sync::Arc;

fn setup_salsa_db(
    workspace: &smelt_bench::model_gen::GeneratedWorkspace,
) -> (smelt_db::Database, Vec<PathBuf>) {
    let mut db = smelt_db::Database::default();
    let mut all_paths = Vec::new();

    for (name, content) in &workspace.sql_contents {
        let path = workspace.models_path().join(format!("{}.sql", name));
        db.set_file_text(path.clone(), Arc::new(content.clone()));
        all_paths.push(path.clone());
    }

    db.set_all_files(Arc::new(all_paths.clone()));

    let project_root = workspace.path().to_path_buf();
    for path in &all_paths {
        db.set_file_project_root(path.clone(), project_root.clone());
    }
    db.set_all_project_roots(Arc::new(vec![project_root.clone()]));

    let sources_yml =
        std::fs::read_to_string(workspace.path().join("sources.yml")).unwrap_or_default();
    db.set_project_sources_yaml(project_root, Arc::new(sources_yml));

    (db, all_paths)
}

fn bench_initial_load(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    c.bench_function("salsa_initial_load_2000", |b| {
        b.iter(|| {
            let (db, _) = setup_salsa_db(&workspace);
            let _models = db.all_models();
        })
    });
}

fn bench_leaf_edit(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");
    let (mut db, all_paths) = setup_salsa_db(&workspace);

    // Warm caches
    let _models = db.all_models();
    for path in &all_paths {
        let _diags = db.file_diagnostics(path.clone());
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
            db.set_file_text(
                leaf_path.clone(),
                Arc::new("SELECT 1 AS bench_col\n".to_string()),
            );
            let _diags = db.file_diagnostics(leaf_path.clone());
        })
    });
}

fn bench_full_diagnostics(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");
    let (db, all_paths) = setup_salsa_db(&workspace);

    // Warm caches
    let _models = db.all_models();

    c.bench_function("salsa_full_diagnostics_2000", |b| {
        b.iter(|| {
            for path in &all_paths {
                let _diags = db.file_diagnostics(path.clone());
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
