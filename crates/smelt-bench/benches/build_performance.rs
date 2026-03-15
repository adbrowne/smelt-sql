use criterion::{criterion_group, criterion_main, Criterion};
use smelt_bench::model_gen::{generate_workspace, GraphSpec};

fn bench_discovery(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    c.bench_function("discovery_2000_models", |b| {
        b.iter(|| {
            let discovery = smelt_core::ModelDiscovery::new(
                workspace.path().to_path_buf(),
                vec!["models".to_string()],
            );
            discovery.discover_models().unwrap();
        })
    });
}

fn bench_graph_build(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    // Pre-discover models
    let discovery =
        smelt_core::ModelDiscovery::new(workspace.path().to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();

    let sources_config = smelt_core::SourcesConfig::load(workspace.path()).unwrap();

    c.bench_function("graph_build_2000_models", |b| {
        b.iter(|| {
            let graph =
                smelt_core::DependencyGraph::build(models.clone(), Some(&sources_config)).unwrap();
            graph.validate().unwrap();
        })
    });
}

fn bench_topo_sort(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    let discovery =
        smelt_core::ModelDiscovery::new(workspace.path().to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();

    let sources_config = smelt_core::SourcesConfig::load(workspace.path()).unwrap();

    let graph = smelt_core::DependencyGraph::build(models, Some(&sources_config)).unwrap();

    c.bench_function("topo_sort_2000_models", |b| {
        b.iter(|| {
            graph.execution_order().unwrap();
        })
    });
}

fn bench_full_build_pipeline(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    c.bench_function("full_build_pipeline_2000_models", |b| {
        b.iter(|| {
            smelt_bench::harness::build_bench::run_build_benchmark(&workspace).unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_discovery,
    bench_graph_build,
    bench_topo_sort,
    bench_full_build_pipeline
);
criterion_main!(benches);
