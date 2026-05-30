# crates/smelt-planner/CLAUDE.md

Planning rules and logical plan types — temporal analysis, batch-safety derivation, incremental materialization rules, cumulative aggregate classification, and the lowering pass (e.g. `as_struct`). Pure Rust; no Salsa dependency.

## How to test

```bash
cargo test -p smelt-planner
```

Integration-level planner tests live in `smelt-cli/tests/planner_test.rs` and `web_analytics_pushdown.rs`, since they require a full project to plan against.

## Gotchas

- **No Salsa dependency.** `smelt-planner` operates on `ModelGraph` and `ModelInfo` structs (plain data), not Salsa tracked inputs. The Salsa query that invokes the planner lives in `smelt-db`. Keep it that way — analysis logic stays pure.
- **`rules/incremental.rs`** derives `BatchSafety` and `derive_model_source_bounds`. These are the entry points for incremental model planning. `BatchSafety` has three variants (`FullyBatchSafe`, `BoundedSafe`, `Unbounded`) — callers branch on them to decide chunk sizes.
- **`rules/cumulative.rs`** classifies cumulative aggregates (`classify_cumulative`) and derives the cross-partition combiner. Cumulative models are a separate execution mode from incremental models.
- **`analysis/temporal.rs`** owns `analyze_temporal_dependencies` and `compute_effective_window`. These compute the temporal dependency graph that feeds both the incremental and cumulative rules.
- **`lowering/as_struct.rs`** rewrites `as_struct(...)` calls during the lowering pass. It's separate from the planner rules because it is shape-preserving (not an optimization opportunity).
- **`python_bridge.rs`** is gated on `#[cfg(feature = "python")]` — it exposes planner types to Python via PyO3.

## Where things live

- `src/rules/` — planning rules: `incremental.rs`, `cumulative.rs`, `cube_split.rs`
- `src/analysis/` — temporal dependency analysis and source-bound derivation
- `src/logical.rs` — `LogicalNode` enum, `Plan` type, `ProvenanceTag`
- `src/lowering/` — lowering passes (`as_struct.rs`)
- `src/graph.rs` — `ModelGraph`, `ModelInfo` (input to all rules)
