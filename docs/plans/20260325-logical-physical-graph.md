# Logical Graph → Physical Graph: Two-Stage Graph Architecture

**Date**: 2025-03-25

## Context

smelt's planner rules can create intermediate tables, inline ephemeral models, fuse models, and redirect references — but there's no clean boundary between "what the user authored" and "what gets executed." The current `DependencyGraph` serves both purposes, and the `smelt-cli` crate has a near-duplicate with extra fields. Planner transformations (`Transformation` enum) produce instructions but don't modify the graph structure itself.

This plan introduces a **two-stage graph architecture** — a Logical Graph (user intent) and a Physical Graph (execution plan) — with a well-defined transition between them. This is the same pattern used by DataFusion (LogicalPlan → ExecutionPlan), Spark Catalyst, and Apache Calcite, adapted to smelt's model-level granularity rather than query-level.

**Goal**: Create a clear contract so that planner rule authors know exactly what they're working with (logical graph in, transformations out, physical graph produced).

## Design

### The Two Graphs

**Logical Graph** — what users author:
- 1:1 with source files (every `.sql` model is a node)
- Includes all models regardless of materialization (table, view, ephemeral, etc.)
- Edges are `smelt.ref()` dependencies
- Used for: validation, model selection (`--select`), lineage, LSP, documentation
- Each node eagerly resolves its config cascade (SQL metadata > smelt.yml > default)

**Physical Graph** — what gets executed:
- Ephemeral nodes removed (their SQL inlined as CTEs into dependents)
- Planner-created intermediates are first-class nodes (shared sub-expressions, incremental staging tables, cube split temps)
- Each node carries a concrete execution strategy (CreateTable, CreateView, Incremental, MultiStep)
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

### Ephemeral Model Handling

This plan is designed independently of the ephemeral models session. The architecture accommodates ephemeral naturally:

- **Where ephemeral fits**: When `Materialization::Ephemeral` exists, the `PhysicalGraphBuilder` inlines those models as CTEs during the logical-to-physical transition
- **Cross-backend**: Ephemeral models have no backend. When inlined into a dependent on backend X, the SQL is compiled with backend X's dialect. If referenced by models on different backends, it's compiled separately per dependent
- **Multiple references**: If ephemeral model E is referenced by models A and B, E's SQL becomes a CTE in both A and B independently (same as dbt)
- **Lineage**: The physical graph keeps an `Inlined` node for ephemeral models so lineage tracking still works

### Rule Interaction

Rules are applied heuristically in a fixed phase order (no cost model). Cross-model rules run first because they need full graph visibility; single-model rules run second and may operate on nodes created in phase 1.

**Open question for future work**: How should rules compose when they conflict? For example, if a cross-model rule wants to materialize a shared sub-expression, but a single-model rule wants to make the same model incremental. Current approach: rules are independent and the last-applied transformation wins. This needs more thought as rule complexity grows -- potential approaches include rule priority ordering, conflict detection with user resolution, or a constraint-based system.

### Validation Split

**Logical graph** (before transformation):
- All refs resolve to models or sources
- No circular dependencies
- Frontmatter validity
- Ephemeral models cannot be incremental
- Ephemeral models that are leaf nodes produce a warning

**Physical graph** (after transformation):
- No cross-backend references remain
- All synthetic node dependencies exist
- No cycles introduced by transformations
- Incremental models are tables (not views)

### Model Selection to Physical Nodes

Users select logical model names via `--select`. Mapping:
1. `LogicalGraph.select_models()` produces a set of logical names (unchanged)
2. `PhysicalGraph.filter_for_selection(logical_names)` returns physical nodes whose `logical_origins` intersect with the selected set
3. Synthetic intermediates are included if their origin model is selected
4. Selecting an ephemeral model directly produces a clear error message

### Naming Synthetic Nodes

Deterministic names: `__smelt__{origin}__{purpose}__{content_hash_8}` (e.g., `__smelt__daily_revenue__cube_tmp__a3f2b1c9`). Content hash ensures stability across runs when SQL is unchanged.

## Implementation Phases

### Phase A: Consolidate graph types
1. Merge smelt-cli's `DependencyGraph`/`ModelFile` extensions into smelt-core
2. Rename `DependencyGraph` to `LogicalGraph`
3. Add `LogicalNode` with eagerly-resolved config cascade
4. Update all consumers (cli, planner) to use the unified type

### Phase B: Introduce PhysicalGraph
1. Create `smelt-core/src/physical_graph.rs` with types above
2. Create `PhysicalGraphBuilder` -- initially 1:1 mapping (no transformations)
3. Wire executor to consume `PhysicalGraph` instead of `LogicalGraph`
4. Existing behavior preserved, just going through the new layer

### Phase C: Wire up planner transformations
1. Add new `Transformation` variants (CreateNode, RemoveNode, RedirectRef, SetMaterialization)
2. `PhysicalGraphBuilder` applies transformations during construction
3. Add ephemeral inlining step (when Ephemeral materialization exists)

### Phase D: Explain output
1. Add `physical_graph` section to `smelt explain` showing nodes, strategies, origins
2. Show which transformations were applied

## Key Files

| File | Change |
|------|--------|
| `crates/smelt-core/src/graph.rs` | Rename DependencyGraph to LogicalGraph, add LogicalNode |
| `crates/smelt-core/src/physical_graph.rs` | **New** -- PhysicalGraph, PhysicalNode, PhysicalStrategy, PhysicalGraphBuilder |
| `crates/smelt-core/src/config.rs` | Materialization enum (add Ephemeral when ready) |
| `crates/smelt-planner/src/types.rs` | Extend Transformation enum with graph-level variants |
| `crates/smelt-planner/src/rules/mod.rs` | Phased rule execution (cross-model then single-model) |
| `crates/smelt-cli/src/main.rs` | Executor rewrite to consume PhysicalGraph |
| `crates/smelt-cli/src/executor.rs` | Executor rewrite to consume PhysicalGraph |

## Prior Art

| System | Pattern | Key Lesson |
|--------|---------|------------|
| DataFusion | LogicalPlan to ExecutionPlan | Clean separation; physical optimizer rules as secondary pass |
| Spark Catalyst | 4-phase pipeline (analysis, logical opt, physical, codegen) | Cost-based only at physical plan selection |
| Apache Calcite | Convention trait on RelNode; VolcanoPlanner | Rules match tree patterns; equivalence classes avoid re-exploration |
| dbt | Ephemeral = CTE inlining | Simple but no synthetic intermediates; duplicates SQL for multi-ref |
| Presto | CTE materialization heuristic (4+ refs AND complex ops) | Cost model should drive materialization decisions |
