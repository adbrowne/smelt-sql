use serde::Deserialize;
use serde_json::json;
use std::path::Path;

/// Minimal struct matching the benchmark result JSON.
/// We only deserialize what we need, avoiding a dependency on the full smelt-bench lib
/// (which pulls in heavy crates like salsa, rowan, etc.).
#[derive(Deserialize)]
struct BenchmarkResult {
    build: BuildMetrics,
    salsa: SalsaMetrics,
    parser: ParserMetrics,
}

#[derive(Deserialize)]
struct BuildMetrics {
    total_ms: f64,
    discovery_ms: f64,
    graph_build_ms: f64,
    topo_sort_ms: f64,
    validation_ms: f64,
}

#[derive(Deserialize)]
struct SalsaMetrics {
    initial_load_ms: f64,
    leaf_edit_diagnostics_ms: f64,
    mid_edit_diagnostics_ms: f64,
    root_edit_diagnostics_ms: f64,
    add_file_all_models_ms: f64,
    full_diagnostics_ms: f64,
}

#[derive(Deserialize)]
struct ParserMetrics {
    single_simple_us: f64,
    single_complex_us: f64,
    batch_all_ms: f64,
    bytes_per_second: f64,
    batch_count: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let results_dir = Path::new("benchmarks/results");

    // Find the latest result file by filename sort
    let mut files: Vec<_> = std::fs::read_dir(results_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();

    files.sort_by_key(|e| e.file_name());

    let latest = files
        .last()
        .ok_or("No benchmark result files found in benchmarks/results/")?;

    eprintln!("Reading {}", latest.path().display());

    let content = std::fs::read_to_string(latest.path())?;
    let result: BenchmarkResult = serde_json::from_str(&content)?;

    let b = &result.build;
    let s = &result.salsa;
    let p = &result.parser;

    // Latency metrics (smaller is better)
    let latency = json!([
        { "name": "Build / Total", "unit": "ms", "value": b.total_ms },
        { "name": "Build / Discovery", "unit": "ms", "value": b.discovery_ms },
        { "name": "Build / Graph Build", "unit": "ms", "value": b.graph_build_ms },
        { "name": "Build / Topo Sort", "unit": "ms", "value": b.topo_sort_ms },
        { "name": "Build / Validation", "unit": "ms", "value": b.validation_ms },
        { "name": "Salsa / Initial Load", "unit": "ms", "value": s.initial_load_ms },
        { "name": "Salsa / Leaf Edit Diagnostics", "unit": "ms", "value": s.leaf_edit_diagnostics_ms },
        { "name": "Salsa / Mid Edit Diagnostics", "unit": "ms", "value": s.mid_edit_diagnostics_ms },
        { "name": "Salsa / Root Edit Diagnostics", "unit": "ms", "value": s.root_edit_diagnostics_ms },
        { "name": "Salsa / Add File", "unit": "ms", "value": s.add_file_all_models_ms },
        { "name": "Salsa / Full Diagnostics", "unit": "ms", "value": s.full_diagnostics_ms },
        { "name": "Parser / Simple SQL", "unit": "μs", "value": p.single_simple_us },
        { "name": "Parser / Complex SQL", "unit": "μs", "value": p.single_complex_us },
        { "name": format!("Parser / Batch ({})", p.batch_count), "unit": "ms", "value": p.batch_all_ms },
    ]);

    // Throughput metrics (bigger is better)
    let throughput = json!([
        { "name": "Parser / Throughput", "unit": "MB/s", "value": p.bytes_per_second / 1_000_000.0 },
    ]);

    // Write output files
    let output_dir = Path::new("benchmarks/output");
    std::fs::create_dir_all(output_dir)?;

    let latency_path = output_dir.join("latency.json");
    std::fs::write(&latency_path, serde_json::to_string_pretty(&latency)?)?;
    eprintln!("Wrote {}", latency_path.display());

    let throughput_path = output_dir.join("throughput.json");
    std::fs::write(&throughput_path, serde_json::to_string_pretty(&throughput)?)?;
    eprintln!("Wrote {}", throughput_path.display());

    Ok(())
}
