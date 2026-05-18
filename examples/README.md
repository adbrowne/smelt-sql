# smelt Examples

## Directories

| Directory | Description | Models |
|-----------|-------------|--------|
| `timeseries/` | User/event analytics pipeline with incremental materialization | 12 SQL |
| `retail_analytics/` | TPC-DS-based retail pipeline (staging/intermediate/marts) | 25 SQL |
| `web_analytics/` | Bronze→silver→gold pipeline over JSON events with three parallel identity-resolution algorithms compared side-by-side | 10 SQL models + 2 functions + 5 tests |
| `broken/` | Intentionally broken models for testing error handling | 5 SQL |
| `test_workspace/` | Minimal workspace for VSCode/LSP integration testing | 7 SQL + 3 Python |
| `huge/` | Auto-generated 2000-model stress test workspace | 1000 SQL + 1000 Python |

## Quick Start

```bash
# Run the timeseries example
cargo run -p smelt-cli -- run --project-dir examples/timeseries

# Validate retail analytics
cargo run -p smelt-cli -- run --project-dir examples/retail_analytics --dry-run

# Regenerate the huge workspace
cargo run -p smelt-bench --bin generate_static_workspace
```
