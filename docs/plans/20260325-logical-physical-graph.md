# Logical Graph → Physical Graph: Two-Stage Graph Architecture

**Date**: 2026-03-25 (updated 2026-03-26 after ephemeral support landed)

## Context

smelt's planner rules can create intermediate tables, inline ephemeral models, fuse models, and redirect references -- but there's no clean boundary between "what the user authored" and "what gets executed." The current `DependencyGraph` serves both purposes, and the `smelt-cli` crate has a near-duplicate with extra fields. Planner transformations (`Transformation` enum) produce instructions but don't modify the graph structure itself.

This plan introduces a **two-stage graph architecture** -- a Logical Graph (user intent) and a Physical Graph (execution plan) -- with a well-defined transition between them. This is the same pattern used by DataFusion (LogicalPlan to ExecutionPlan), Spark Catalyst, and Apache Calcite, adapted to smelt's model-level granularity rather than query-level.

**Goal**: Create a clear contract so that planner rule authors know exactly what they're working with (logical graph in, transformations out, physical graph produced).

### What already exists (as of 2026-03-26)

The ephemeral model support has landed on main. Key pieces we can build on:

- **`Materialization` enum** now has `Table`, `View`, `Ephemeral`, `MaterializedView` (`smelt-core/src/config.rs`)
- **`EphemeralResolver`** in `smelt-cli/src/compiler.rs` -- already handles CTE inlining with:
  - Topological ordering of ephemeral dependencies
  - CTE namespace deduplication (`__smelt_{model}` naming)
  - Internal CTE hoisting and namespacing (`__smelt_{model}__{cte_name}`)
  - Flat hoisting strategy (Spark-safe, no nested CTEs)
- **Validation** already in place:
  - `Config::validate_model_configs()` -- ephemeral+incremental error, ephemeral+target error
  - `graph.warn_unused_ephemerals()` -- warns about leaf ephemeral models
  - Selector validation -- error when `--select` targets an ephemeral directly
- **CLI execution loop** skips ephemeral models and passes `EphemeralResolver` to `compile_with_ephemerals()`
- **smelt-cli still has its own `DependencyGraph`** in `smelt-cli/src/graph.rs` (duplicate of smelt-core's)

## Design

### The Two Graphs

**Logical Graph** -- what users author:
- 1:1 with source files (every `.sql` model is a node)
- Includes all models regardless of materialization (table, view, ephemeral, etc.)
- Edges are `smelt.ref()` dependencies
- Used for: validation, model selection (`--select`), lineage, LSP, documentation
- Each node eagerly resolves its config cascade (SQL metadata > smelt.yml > default)

**Physical Graph** -- what gets executed:
- Ephemeral nodes removed (their SQL inlined as CTEs into dependents via existing `EphemeralResolver`)
- Planner-created intermediates are first-class nodes (shared sub-expressions, incremental staging tables, cube split temps)
- Each node carries a concrete execution strategy (CreateTable, CreateView, CreateMaterializedView, Incremental, MultiStep)
- The executor operates exclusively on this graph

### The Transition Pipeline

```
LogicalGraph
    |
    +- validate (refs, cycles, config)
    |
    +- Phase 1: Cross-model rules (shared materialization, model fusion)
    +- Phase 2: Single-model rules (cube split, incremental detection)
    |
    +- Apply transformations
    +- Inline ephemeral models as CTEs
    +- Resolve execution strategies
    +- Topological sort
    |
    +-> PhysicalGraph
```

### Key Types

#### LogicalGraph (consolidation of existing DependencyGraph)

```rust
// smelt-core/src/graph.rs -- replaces DependencyGraph

pub struct LogicalGraph {
    nodes: HashMap<String, LogicalNode>,
    sources: HashSet<String>,
}

pub struct LogicalNode {
    pub name: String,
    pub model_file: ModelFile,
    pub dependencies: Vec<String>,
    pub materialization: Materialization,   // resolved from config cascade
    pub incremental: Option<IncrementalConfig>,
    pub target: String,                     // resolved target name
    pub tags: Vec<String>,
}
```

This consolidates the smelt-core `DependencyGraph` and smelt-cli's duplicate. Config cascade resolution (SQL metadata > smelt.yml > default) happens at construction time, not scattered through consumers.

#### PhysicalGraph

```rust
// smelt-core/src/physical_graph.rs (new file)

pub enum PhysicalNodeId {
    Model(String),                              // from a user-authored model
    Synthetic { name: String, origin: String },  // planner-created
}

pub enum PhysicalStrategy {
    CreateTable { sql: String },
    CreateView { sql: String },
    CreateMaterializedView { sql: String },
    Incremental { sql: String, config: IncrementalConfig, strategy: IncrementalStrategy },
    MultiStep { steps: Vec<ExecutionStep> },
    Inlined,  // ephemeral -- kept for lineage, skipped by executor
}

pub struct PhysicalNode {
    pub id: PhysicalNodeId,
    pub dependencies: Vec<PhysicalNodeId>,
    pub strategy: PhysicalStrategy,
    pub target: String,
    pub logical_origins: Vec<String>,  // traces back to user-authored model(s)
}

pub struct PhysicalGraph {
    nodes: HashMap<PhysicalNodeId, PhysicalNode>,
    execution_order: Vec<PhysicalNodeId>,  // topologically sorted, excludes Inlined
}
```

#### Extended Transformations

```rust
// smelt-planner/src/types.rs -- extend existing enum

pub enum Transformation {
    // Existing:
    ReplaceWithPlan { model, steps },
    SetIncremental { model, event_time_column, partition_column, granularity },
    // New graph-level:
    CreateNode { name, sql, dependencies, origin, materialization },
    RemoveNode { model },
    RedirectRef { from, to },
    SetMaterialization { model, materialization },
}
```

### Ephemeral Model Handling (already implemented -- to be absorbed)

Ephemeral support has landed. The `PhysicalGraphBuilder` reuses the existing `EphemeralResolver` (`smelt-cli/src/compiler.rs`) during the logical-to-physical transition:

- **CTE inlining**: `EphemeralResolver` already handles topological ordering, namespace deduplication (`__smelt_{model}`), internal CTE hoisting, and flat hoisting (Spark-safe)
- **Cross-backend**: Ephemeral models inherit the dependent's backend dialect. Multi-backend refs compile separately per dependent -- already handled by `PrintContext.ephemeral_models`
- **Multiple references**: Each dependent gets its own copy of the ephemeral CTE (same as dbt)
- **Lineage**: The physical graph keeps an `Inlined` node for ephemeral models so lineage tracking still works
- **Validation**: `Config::validate_model_configs()` already rejects ephemeral+incremental, ephemeral+target. `warn_unused_ephemerals()` catches leaf ephemerals. Selector validation prevents `--select` on ephemerals.

The key refactoring opportunity: the current ephemeral logic is spread across `main.rs` (resolver construction, skip logic) and `compiler.rs` (EphemeralResolver). The `PhysicalGraphBuilder` centralizes this -- ephemeral inlining becomes a step in the logical-to-physical transition rather than ad-hoc logic in the execution loop.

### Rule Interaction

Rules are applied heuristically in a fixed phase order (no cost model). Cross-model rules run first because they need full graph visibility; single-model rules run second and may operate on nodes created in phase 1.

**Open question for future work**: How should rules compose when they conflict? For example, if a cross-model rule wants to materialize a shared sub-expression, but a single-model rule wants to make the same model incremental. Current approach: rules are independent and the last-applied transformation wins. This needs more thought as rule complexity grows -- potential approaches include rule priority ordering, conflict detection with user resolution, or a constraint-based system.

### Validation Split

**Logical graph** (before transformation) -- mostly already implemented:
- All refs resolve to models or sources *(existing: `DependencyGraph::validate()`)*
- No circular dependencies *(existing: `execution_order()` detects cycles)*
- Frontmatter validity *(existing: metadata parsing)*
- Ephemeral models cannot be incremental *(existing: `validate_model_configs()`)*
- Ephemeral models that are leaf nodes produce a warning *(existing: `warn_unused_ephemerals()`)*
- MaterializedView+incremental produces a warning *(existing: `validate_model_configs()`)*

**Physical graph** (after transformation):
- No cross-backend references remain
- All synthetic node dependencies exist
- No cycles introduced by transformations
- Incremental models are tables (not views or materialized views)

### Model Selection to Physical Nodes

Users select logical model names via `--select`. Mapping:
1. `LogicalGraph.select_models()` produces a set of logical names (unchanged)
2. `PhysicalGraph.filter_for_selection(logical_names)` returns physical nodes whose `logical_origins` intersect with the selected set
3. Synthetic intermediates are included if their origin model is selected
4. Selecting an ephemeral model directly produces a clear error message

### Naming Synthetic Nodes

Deterministic names: `__smelt__{origin}__{purpose}__{content_hash_8}` (e.g., `__smelt__daily_revenue__cube_tmp__a3f2b1c9`). Content hash ensures stability across runs when SQL is unchanged.

## Implementation Phases

### Phase A: Consolidate graph types ✅ (March 26, 2026)
1. ✅ Added missing methods (`models()`, `get_upstream()`, `all_upstream()`, `warn_unused_ephemerals()`) to smelt-core's `DependencyGraph`
2. ✅ Created `LogicalGraph` and `LogicalNode` in `smelt-cli/src/logical_graph.rs` (lives in smelt-cli due to `ModelFile`/`ModelKind` type difference between smelt-core and smelt-cli)
3. ✅ `LogicalNode` eagerly resolves config cascade: materialization, target, incremental, tags
4. ✅ Migrated `run()`, `backbuild()`, `explain()` in main.rs to use `LogicalGraph`
5. ✅ Migrated `backfill.rs` and `explain.rs` -- dropped `Config` param from `compute_backbuild_plans`, `compute_range_run_plans`, `build_explain_output`
6. ✅ `select_models()`, `exclude_models()`, `warn_unused_ephemerals()`, `validate_cross_backend_refs()` no longer require `Config` param
7. ⏸️ Old `smelt-cli/src/graph.rs` kept for now (still used by `python.rs` tests); smelt-core `DependencyGraph` kept for smelt-ui

### Phase B: Introduce PhysicalGraph ✅ (March 26, 2026)
1. ✅ Created `smelt-cli/src/physical_graph.rs` with `PhysicalStrategy`, `PhysicalNode`, `PhysicalGraph`, `PhysicalGraphBuilder` (lives in smelt-cli due to `ModelFile`/`CompilerRegistry`/`EphemeralResolver` dependencies)
2. ✅ `PhysicalGraphBuilder::build()` absorbs three blocks from `run()`: transformation parsing, ephemeral resolver construction, per-model strategy resolution
3. ✅ `PhysicalGraph` owns `EphemeralResolver`s per target -- ephemerals filtered out of execution order
4. ✅ `run()` execution loop iterates `physical_graph.iter_in_order()` instead of raw execution_order
5. ✅ Deleted `ModelExecution` enum -- replaced by `PhysicalStrategy`
6. ✅ 8 unit tests covering strategy resolution, ephemeral filtering, resolver construction
7. ⏸️ `backbuild()` not yet migrated -- uses its own execution pattern, deferred to follow-up

### Phase C: Wire up planner transformations ✅ (March 26, 2026)
1. ✅ Added 4 new `Transformation` variants to `smelt-planner/src/types.rs`: `CreateNode`, `RemoveNode`, `RedirectRef`, `SetMaterialization`
2. ✅ `PhysicalGraphBuilder::build()` applies all transformation types during construction:
   - `CreateNode`: adds synthetic `PhysicalNode`s with correct dependency ordering and `logical_origins`
   - `RemoveNode`: excludes models from physical graph
   - `RedirectRef`: rewrites `smelt.ref()` calls in model SQL content
   - `SetMaterialization`: overrides materialization from logical graph
3. ✅ `validate_transformations()` checks all graph-level transformations before applying (unknown models, name conflicts, missing dependencies)
4. ✅ `PhysicalNode` now carries `logical_origins` field for tracing back to user-authored models
5. ✅ `PhysicalGraph::filter_for_selection()` maps `--select` logical names to physical nodes (includes synthetic intermediates)
6. ✅ Strategy resolution extracted into `resolve_strategy()` and `resolve_plan_steps()` helper methods
7. ✅ 18 unit tests (8 existing + 10 new covering all transformation types and validation errors)

### Phase D: Explain output
1. Add `physical_graph` section to `smelt explain` showing nodes, strategies, origins
2. Show which transformations were applied

## Key Files

| File | Change |
|------|--------|
| `crates/smelt-core/src/graph.rs` | Absorb smelt-cli extensions, rename DependencyGraph to LogicalGraph, add LogicalNode |
| `crates/smelt-core/src/physical_graph.rs` | **New** -- PhysicalGraph, PhysicalNode, PhysicalStrategy, PhysicalGraphBuilder |
| `crates/smelt-core/src/config.rs` | Already has Ephemeral/MaterializedView -- no changes needed |
| `crates/smelt-cli/src/graph.rs` | **Delete** -- consolidated into smelt-core |
| `crates/smelt-cli/src/compiler.rs` | `EphemeralResolver` stays here, called by PhysicalGraphBuilder |
| `crates/smelt-cli/src/main.rs` | Simplify: build LogicalGraph to PhysicalGraph to execute. Remove ad-hoc ephemeral/materialization logic |
| `crates/smelt-cli/src/executor.rs` | Executor consumes PhysicalGraph |
| `crates/smelt-planner/src/types.rs` | Extend Transformation enum with graph-level variants |
| `crates/smelt-planner/src/rules/mod.rs` | Phased rule execution (cross-model then single-model) |

## Prior Art

| System | Pattern | Key Lesson |
|--------|---------|------------|
| DataFusion | LogicalPlan to ExecutionPlan | Clean separation; physical optimizer rules as secondary pass |
| Spark Catalyst | 4-phase pipeline (analysis, logical opt, physical, codegen) | Cost-based only at physical plan selection |
| Apache Calcite | Convention trait on RelNode; VolcanoPlanner | Rules match tree patterns; equivalence classes avoid re-exploration |
| dbt | Ephemeral = CTE inlining | Simple but no synthetic intermediates; duplicates SQL for multi-ref |
| Presto | CTE materialization heuristic (4+ refs AND complex ops) | Cost model should drive materialization decisions |
