//! Binary that runs all benchmarks and saves combined results as JSON.
//!
//! This is meant to be run after `cargo bench -p smelt-bench` in CI,
//! or standalone to produce a results file.

use anyhow::Result;
use smelt_bench::model_gen::{generate_workspace, GraphSpec};
use smelt_bench::BenchmarkResult;
use std::path::PathBuf;

fn main() -> Result<()> {
    eprintln!("Generating benchmark workspace (2000 models)...");
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec)?;

    eprintln!("Running build benchmark...");
    let build = smelt_bench::harness::build_bench::run_build_benchmark(&workspace)?;
    eprintln!(
        "  Discovery: {:.1}ms, Graph: {:.1}ms, Topo: {:.1}ms",
        build.discovery_ms, build.graph_build_ms, build.topo_sort_ms
    );

    eprintln!("Running Salsa benchmark...");
    let salsa = smelt_bench::harness::salsa_bench::run_salsa_benchmark(&workspace)?;
    eprintln!(
        "  Initial load: {:.1}ms, Full diagnostics: {:.1}ms",
        salsa.initial_load_ms, salsa.full_diagnostics_ms
    );

    eprintln!("Running parser benchmark...");
    let parser = smelt_bench::harness::parser_bench::run_parser_benchmark(&workspace);
    eprintln!(
        "  Simple: {:.1}μs, Complex: {:.1}μs, Throughput: {:.1} MB/s",
        parser.single_simple_us,
        parser.single_complex_us,
        parser.bytes_per_second / 1_000_000.0
    );

    let result = BenchmarkResult {
        git_commit: smelt_bench::metrics::git_commit(),
        git_branch: smelt_bench::metrics::git_branch(),
        timestamp: smelt_bench::metrics::timestamp(),
        rust_version: smelt_bench::metrics::rust_version(),
        model_count: spec.total_models(),
        build,
        salsa,
        parser,
    };

    let results_dir = PathBuf::from("benchmarks/results");
    result.save_to_dir(&results_dir)?;

    // Also print to stdout as JSON
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
