use crate::model_gen::GeneratedWorkspace;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Metrics from a build pipeline benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMetrics {
    /// Total wall-clock time in milliseconds.
    pub total_ms: f64,
    /// Time to discover models (walk filesystem + parse).
    pub discovery_ms: f64,
    /// Time to build the dependency graph.
    pub graph_build_ms: f64,
    /// Time for topological sort.
    pub topo_sort_ms: f64,
    /// Time for graph validation.
    pub validation_ms: f64,
    /// Number of models discovered.
    pub model_count: usize,
}

/// Run the build pipeline benchmark on a generated workspace.
///
/// Measures: discovery → graph build → validation → topo sort.
/// Does NOT include DuckDB execution (I/O bound, noisy).
pub fn run_build_benchmark(workspace: &GeneratedWorkspace) -> Result<BuildMetrics> {
    let total_start = Instant::now();

    // Phase 1: Model discovery
    let disc_start = Instant::now();
    let discovery =
        smelt_core::ModelDiscovery::new(workspace.path().to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models()?;
    let discovery_ms = disc_start.elapsed().as_secs_f64() * 1000.0;

    let model_count = models.len();

    // Phase 2: Parse sources config
    let sources_config = smelt_core::SourcesConfig::load(workspace.path())?;

    // Phase 3: Build dependency graph
    let graph_start = Instant::now();
    let graph = smelt_core::DependencyGraph::build(models, Some(&sources_config))?;
    let graph_build_ms = graph_start.elapsed().as_secs_f64() * 1000.0;

    // Phase 4: Validate graph
    let validate_start = Instant::now();
    graph.validate()?;
    let validation_ms = validate_start.elapsed().as_secs_f64() * 1000.0;

    // Phase 5: Topological sort
    let topo_start = Instant::now();
    let _order = graph.execution_order()?;
    let topo_sort_ms = topo_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

    Ok(BuildMetrics {
        total_ms,
        discovery_ms,
        graph_build_ms,
        topo_sort_ms,
        validation_ms,
        model_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gen::{generate_workspace, GraphSpec};

    #[test]
    fn test_build_benchmark_small() {
        let spec = GraphSpec::small();
        let workspace = generate_workspace(&spec).unwrap();
        let metrics = run_build_benchmark(&workspace).unwrap();

        // Small spec generates 20 models total, but only SQL models are discovered
        // (Python models are .py files, discovery only finds .sql)
        assert!(metrics.model_count > 0);
        assert!(metrics.total_ms > 0.0);
        assert!(metrics.discovery_ms > 0.0);
    }
}
