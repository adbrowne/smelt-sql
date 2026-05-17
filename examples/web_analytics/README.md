# Web analytics example

Synthetic web-analytics example demonstrating a bronze→silver pipeline over JSON-encoded events. See [the overall plan](../../docs/plans/20260517-web-analytics-example.md) for the full build-out roadmap.

To run locally:

```bash
smelt-datagen --config datagen.yaml --scale-factor 0.01
duckdb target/dev.duckdb < setup_sources.sql
smelt build
```
