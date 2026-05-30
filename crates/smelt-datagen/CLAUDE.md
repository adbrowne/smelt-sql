# crates/smelt-datagen/CLAUDE.md

Deterministic test-data generator — a standalone binary (`smelt-datagen`) that produces reproducible CSV and Parquet datasets for pipeline testing. All output is seeded via `ChaCha` RNG so the same config always produces the same rows.

## How to test

```bash
cargo test -p smelt-datagen
```

## Gotchas

- **Binary-only crate.** `smelt-datagen` exposes a library surface so `dev-dependencies` (DuckDB, `smelt-parser`) can consume it in tests, but there is no public API intended for other crates to depend on at runtime.
- **Two output modes.** `generic.rs` writes CSV via Arrow in-memory; `generic_parquet.rs` writes Hive-partitioned Parquet files (2000+ lines combined). Both share the generator/config stack but differ in how they materialise rows to disk. New output formats follow the same pattern.
- **Seeded RNG is mandatory for reproducibility.** All generators consume a `ChaCha` seed derived from the config. Do not introduce `thread_rng()` or any non-seeded source — it breaks determinism.
- **Config-driven generation.** `config.rs` (1200+ lines) owns YAML deserialization for the full generator configuration. The `--list-generators` CLI flag prints all generator types and their YAML parameters without running generation — use it to discover available parameters before editing config files.

## Where things live

- `src/main.rs` — CLI entry point (`clap`-based), wires config → generator → output
- `src/config.rs` — YAML config deserialization; generator parameter types
- `src/gen.rs` — `Gen` trait: the core abstraction all generators implement
- `src/generators.rs` — primitive generators (uniform, geometric, categorical, etc.)
- `src/session.rs` — `SessionGenerator`, `VisitorPool`, `DayGenerator` — composable session-level generators used by timeseries fixtures
- `src/generic.rs` — table-level CSV output (Arrow in-memory)
- `src/generic_parquet.rs` — table-level Parquet output (Hive-partitioned)
- `src/parquet.rs` — lower-level Parquet writer helpers
