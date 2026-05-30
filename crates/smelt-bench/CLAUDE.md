# crates/smelt-bench/CLAUDE.md

Benchmarking harness — tools to generate large synthetic workspaces (`model_gen`), measure Salsa incremental recomputation latency (`harness/salsa_bench.rs`), and measure the build pipeline (discovery → graph → validation → sort) without executing SQL.

## How to test

```bash
# Run benchmark library tests
cargo test -p smelt-bench

# Run a specific benchmark binary (outputs JSON to stdout)
cargo run -p smelt-bench --bin save_results
cargo run -p smelt-bench --bin export_action_benchmark
```

Benchmark binaries are in `src/bin/`. They are not criterion benches — they produce `BenchmarkResult` structs serialized to JSON, designed for tracking regressions over time.

## Gotchas

- **`generate_workspace` / `generate_workspace_to_path`** create a fully valid smelt project in a temp directory (or a given path). Use `GraphSpec` to control the model graph shape (depth, branching, SQL/Python mix). The `examples/huge/` workspace was generated this way.
- **Salsa benchmarks measure edit latency, not cold throughput.** `run_salsa_benchmark` warms the cache first (initial load), then measures recomputation after single-file edits at leaf, mid, and root layers of the dependency graph. These are the numbers that matter for LSP responsiveness.
- **Python model generation is included in `model_gen`** but Salsa benchmarks skip Python models (they produce SQL which is what Salsa sees). Benchmark numbers are therefore SQL-only.
- **No criterion.** Benchmarks are manual timing with `std::time::Instant`. If you want micro-benchmark comparison across runs, use the `BenchmarkResult` JSON + `save_results` binary rather than running raw `cargo bench`.
- **`rand_chacha` for determinism.** The workspace generator uses a seeded `ChaCha` RNG so generated workspaces are reproducible across runs given the same seed.

## Where things live

- `src/model_gen/` — synthetic workspace generation (`GraphSpec`, `generate_workspace`)
- `src/harness/` — benchmark harness: `salsa_bench.rs`, `build_bench.rs`, `parser_bench.rs`
- `src/metrics.rs` — `BenchmarkResult` struct and JSON output helpers
- `src/bin/` — runnable benchmark binaries
