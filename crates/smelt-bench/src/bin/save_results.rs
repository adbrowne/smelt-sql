//! Binary that runs all benchmarks and saves combined results as JSON.
//!
//! This is meant to be run after `cargo bench -p smelt-bench` in CI,
//! or standalone to produce a results file.

use anyhow::Result;
use smelt_bench::harness::salsa_bench::SalsaMetrics;
use smelt_bench::model_gen::{generate_workspace, GeneratedWorkspace, GraphSpec};
use smelt_bench::BenchmarkResult;
use std::path::PathBuf;

/// Number of times the Salsa benchmark is repeated; the reported value is the
/// best (minimum) per metric. Single-shot timings on shared CI runners carry
/// ~15% jitter, which let the historical ~100x regression creep in under the
/// dashboard's 150%-consecutive-step alert. Best-of-N collapses that jitter.
const SALSA_REPEATS: usize = 3;

/// Absolute ceiling (ms) for the cold Salsa passes. A guard against
/// *catastrophic* (≈10x-class) regressions — e.g. the O(N^2) project-path
/// blow-up that once drove these passes into the multi-second→multi-hour range.
/// Deliberately generous (several× the healthy value) so normal runner-speed
/// variance never false-fails; override via `SMELT_BENCH_MAX_SALSA_MS`.
const DEFAULT_MAX_SALSA_MS: f64 = 10_000.0;

/// Run the Salsa benchmark `repeats` times and keep the minimum of each metric.
fn run_salsa_best_of(workspace: &GeneratedWorkspace, repeats: usize) -> Result<SalsaMetrics> {
    let mut best = smelt_bench::harness::salsa_bench::run_salsa_benchmark(workspace)?;
    for _ in 1..repeats {
        let m = smelt_bench::harness::salsa_bench::run_salsa_benchmark(workspace)?;
        best.initial_load_ms = best.initial_load_ms.min(m.initial_load_ms);
        best.leaf_edit_diagnostics_ms = best
            .leaf_edit_diagnostics_ms
            .min(m.leaf_edit_diagnostics_ms);
        best.mid_edit_diagnostics_ms = best.mid_edit_diagnostics_ms.min(m.mid_edit_diagnostics_ms);
        best.root_edit_diagnostics_ms = best
            .root_edit_diagnostics_ms
            .min(m.root_edit_diagnostics_ms);
        best.add_file_all_models_ms = best.add_file_all_models_ms.min(m.add_file_all_models_ms);
        best.full_diagnostics_ms = best.full_diagnostics_ms.min(m.full_diagnostics_ms);
    }
    Ok(best)
}

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

    eprintln!("Running Salsa benchmark (best of {SALSA_REPEATS})...");
    let salsa = run_salsa_best_of(&workspace, SALSA_REPEATS)?;
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

    // Catastrophic-regression gate. Runs *after* the result is saved/printed so
    // the data point is still recorded on the dashboard even when the gate
    // trips. See DEFAULT_MAX_SALSA_MS for the rationale and headroom.
    let max_salsa_ms = std::env::var("SMELT_BENCH_MAX_SALSA_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(DEFAULT_MAX_SALSA_MS);
    let regressions: Vec<String> = [
        ("initial_load_ms", result.salsa.initial_load_ms),
        ("full_diagnostics_ms", result.salsa.full_diagnostics_ms),
    ]
    .into_iter()
    .filter(|(_, v)| *v > max_salsa_ms)
    .map(|(name, v)| format!("{name} = {v:.1}ms exceeds ceiling {max_salsa_ms:.0}ms"))
    .collect();
    if !regressions.is_empty() {
        eprintln!(
            "BENCHMARK REGRESSION GATE FAILED:\n  {}\n\
             (cold Salsa pass blew past the catastrophic-regression ceiling; \
             override with SMELT_BENCH_MAX_SALSA_MS if this is an intentional, \
             reviewed change.)",
            regressions.join("\n  ")
        );
        std::process::exit(1);
    }

    Ok(())
}
