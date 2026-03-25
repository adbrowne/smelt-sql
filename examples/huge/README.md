# Huge Example (2000 models)

Auto-generated workspace with 2000 models for stress testing and benchmarking.

## Structure

- 4 layers, each with 250 SQL + 250 Python models
- 20 source tables
- Deterministic generation (seed: `0xDEADBEEF`)

## Regenerate

```bash
cargo run -p smelt-bench --bin generate_static_workspace
```

Or with a custom output directory:

```bash
cargo run -p smelt-bench --bin generate_static_workspace -- /path/to/output
```
