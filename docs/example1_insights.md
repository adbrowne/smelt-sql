# Example 1: Common Intermediate Aggregation - Insights

## Problem Statement

Three analytics models all require session-level aggregation from raw events:
- **Model A**: Daily active sessions by user
- **Model B**: Session metrics by country
- **Model C**: Revenue per session by hour-of-day

**Naive approach**: Each model computes sessions independently (3x redundant work)

**Optimized approach**: Compute session summary once, all models query from it

## Results

Both versions produce identical outputs (correctness preserved):
- Model A: 5 rows (user-day combinations)
- Model B: 3 rows (country aggregations)
- Model C: 5 rows (hourly aggregations)

## Key Insights for Optimizer API

### 1. Pattern Detection Requirements

The optimizer needs to:

- **Detect common subexpressions** across multiple models
  - All three models have identical `event_gaps` CTE
  - All three models have identical `sessions` CTE
  - All three models do initial `GROUP BY user_id, session_id`

- **Identify the materialization point**
  - The `session_summary` CTE is where computations diverge
  - This is the optimal point to materialize shared state

- **Handle structural equivalence**, not just text matching
  - CTEs might have different names but same logic
  - Column order might differ
  - Need semantic equivalence checking

### 2. Schema Inference Challenge

The shared `session_summary` must include **all dimensions** used by downstream models:

```sql
CREATE TEMP TABLE session_summary AS
SELECT
    user_id,
    session_id,
    MIN(event_time) AS session_start_time,
    DATE_TRUNC('day', MIN(event_time)) AS session_day,      -- for Model A
    EXTRACT(HOUR FROM MIN(event_time)) AS session_hour,     -- for Model C
    FIRST(country ORDER BY event_time) AS session_country,  -- for Model B
    COUNT(*) AS events_in_session,
    SUM(revenue) AS session_revenue
FROM sessions
GROUP BY user_id, session_id
```

**Challenge**: In the naive version, none of the individual models compute ALL these fields. The optimizer must:
1. Analyze all consumers (Models A, B, C)
2. Compute the union of required dimensions
3. Ensure the shared table has everything needed

**This is non-trivial** because it requires forward analysis of what each consumer needs.

### 3. Materialization Strategy Decision

When should the shared computation be materialized?

- **Temp Table** (what we used):
  - Pros: Multiple consumers, actual storage, can be indexed
  - Cons: Overhead of materialization, storage cost

- **CTE** (WITH clause):
  - Pros: No storage overhead
  - Cons: Some engines re-compute for each reference

- **View**:
  - Pros: Persistent, can be used across sessions
  - Cons: Not suitable for session-specific aggregations

**Decision factors**:
- Number of consumers (3 in this case → temp table makes sense)
- Data volume (larger → more benefit from materialization)
- Backend capabilities (some optimize CTEs better than others)
- Latency requirements (temp table adds latency for materialization)

### 4. Correctness Preservation

The optimization must guarantee:
- **Semantic equivalence**: Results are bitwise identical to naive version
- **Determinism**: Same inputs always produce same outputs
- **Aggregate correctness**: Particularly for complex window functions

**Testing strategy**:
- Run both naive and optimized versions
- Compare all outputs (already verified - they match!)
- Property-based testing for variations

### 5. Robustness to Model Changes

**Critical question**: What happens when a model changes?

**Scenario A**: Model B adds a new dimension (e.g., `device_type`)
- Optimizer must re-analyze and regenerate `session_summary`
- Need to add `device_type` to the shared schema
- Other models (A, C) are unaffected

**Scenario B**: Model C changes aggregation logic
- If it still needs session-level data → can still share
- If it needs different session definition (e.g., 60-min windows) → can't share anymore
- Optimizer must detect when sharing is no longer valid

**This suggests the API needs**:
- Dependency tracking (which models depend on which shared computations)
- Incremental re-optimization (only regenerate what changed)
- Ability to "break" a shared computation when it's no longer beneficial

## API Design Options

### Option A: Fully Automatic

```rust
// User just defines models, optimizer does everything
model! {
    name: "model_a",
    sql: "SELECT ... FROM events WHERE ..."
}

model! {
    name: "model_b",
    sql: "SELECT ... FROM events WHERE ..."
}

// Optimizer automatically detects common subexpressions
// No user intervention required
```

**Pros**: Zero cognitive overhead for users
**Cons**: "Magic" behavior, hard to debug, unpredictable performance

### Option B: Hints/Annotations

```rust
model! {
    name: "sessions",
    sql: "SELECT ... FROM events ...",
    materialize: Hint::Auto, // or Hint::Always, Hint::Never
}

model! {
    name: "model_a",
    sql: "SELECT ... FROM {{ ref('sessions') }} ...",
}
```

**Pros**: User can guide optimizer without writing optimization rules
**Cons**: Still somewhat magical, hints might be ignored

### Option C: Explicit Optimization Rules

```rust
optimization_rule! {
    name: "share_session_aggregation",

    // Pattern: detect models that compute similar session aggregations
    pattern: |ctx| {
        ctx.find_models_with_common_cte("sessions")
    },

    // Rewrite: create shared materialization
    rewrite: |ctx, models| {
        let shared_schema = ctx.union_required_columns(models);
        ctx.create_shared_table("session_summary", shared_schema);
        ctx.rewrite_models_to_use_table(models, "session_summary");
    },

    // Cost: only apply if beneficial
    apply_if: |ctx| {
        ctx.num_consumers() >= 2 && ctx.estimated_cost_reduction() > 0
    }
}
```

**Pros**: Explicit, debuggable, powerful, composable
**Cons**: Requires learning the rule API, more verbose

## Recommended Approach

**Start with Option C** (explicit rules) for the API design because:

1. **Clarity**: Users understand exactly what optimizations are happening
2. **Debugging**: When something breaks, you can see which rule caused it
3. **Extensibility**: Users can write custom rules for domain-specific optimizations
4. **Evolution path**: Can add automatic detection later (Option A) as a layer on top

**Future enhancement**: Build a library of common rules that work automatically, giving users Option A for common cases but Option C for custom optimizations.

## Next Steps

1. **Implement Rule API prototype**
   - Define `OptimizationRule` trait
   - Implement pattern matching on SQL AST
   - Implement rewrite mechanism

2. **Test with variations**
   - What if one model needs hourly sessions, another needs daily?
   - What if aggregation functions differ (SUM vs AVG)?
   - How do we handle when sharing is no longer valid?

3. **Build Example 2** (split large GROUP BY)
   - Different optimization pattern
   - Will reveal more API requirements
   - Helps validate that the rule API generalizes
