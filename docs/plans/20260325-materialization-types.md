# Plan: Materialization Types

## Context

smelt currently supports two materialization types: `Table` and `View`. This plan adds `Ephemeral` (CTE-inlined, not materialized) and `MaterializedView` (backend-managed persistent view) as a flat enum with validation rules. Incremental remains orthogonal — it modifies how a `Table` is updated, not a separate materialization type.

The `Materialization` enum exists in two places today:
- `smelt-core::config::Materialization` — config/metadata layer (serde)
- `smelt-backend::types::Materialization` — backend execution layer

Design decision: Add `Ephemeral` only to the core enum (it never reaches the backend). Add `MaterializedView` to both.

---

## Phase 1: Enum Expansion + Validation

No behavior change for existing users. Just widen the type and add validation.

### `crates/smelt-core/src/config.rs`
- Add `Ephemeral` and `MaterializedView` variants to enum
- Update `Deserialize` impl: accept `"ephemeral"` and `"materialized_view"`
- Update `Serialize` impl
- Add `Config::validate_model_config()` method:
  - `Ephemeral` + `incremental` → error
  - `Ephemeral` + `target` override → error
  - `View` + `incremental` → warning (existing behavior, formalize it)
- Add tests

### `crates/smelt-backend/src/types.rs`
- Add `MaterializedView` variant (NOT `Ephemeral` — never reaches backend)
- Update `Display` and `FromStr`

### `crates/smelt-dialect/src/dialect.rs`
- Add `supports_materialized_views: bool` to `BackendCapabilities`
- DuckDB: `false`, Spark: `true`, PostgreSQL: `true`

### `crates/smelt-cli/src/executor.rs`
- Update mapping (lines 18-21):
  - `MaterializedView` → backend `MaterializedView`
  - `Ephemeral` → unreachable/error (should never reach executor)

### `crates/smelt-core/src/metadata.rs`
- Add tests for `materialization: ephemeral` and `materialization: materialized_view` in frontmatter

---

## Phase 2: Ephemeral Model Support

Ephemeral models are compiled into CTEs and inlined into every downstream model that references them.

### CTE Inlining Strategy: Flattened Hoisting with Namespacing

**Critical constraint**: Spark does not support nested CTEs (`WITH a AS (WITH b AS (...) ...) ...`). All CTEs — both ephemeral model bodies and their internal CTEs — must be hoisted into a single flat `WITH` clause.

**Naming convention**: Ephemeral CTEs use `__smelt_{model_name}` prefix. Internal CTEs of ephemeral models use `__smelt_{model_name}__{cte_name}`. This makes collisions with user-defined CTEs structurally impossible.

#### Simple example — B (ephemeral), A (ephemeral, refs B), C (table, refs A and B):

```sql
-- C's compiled output:
WITH __smelt_b AS (
  SELECT * FROM raw_data
), __smelt_a AS (
  SELECT * FROM __smelt_b
)
SELECT * FROM __smelt_a JOIN __smelt_b
```

#### Complex example — ephemeral model with internal CTEs:

```sql
-- Ephemeral model "staging_events" has its own CTE "cleaned":
WITH cleaned AS (SELECT * FROM raw WHERE valid = true)
SELECT * FROM cleaned

-- Downstream model also has CTE named "cleaned":
WITH cleaned AS (SELECT * FROM other_table)
SELECT * FROM smelt.ref('staging_events') JOIN cleaned
```

Compiled output (all flat, Spark-safe, no collisions):
```sql
WITH __smelt_staging_events__cleaned AS (
  SELECT * FROM raw WHERE valid = true
), __smelt_staging_events AS (
  SELECT * FROM __smelt_staging_events__cleaned
), cleaned AS (
  SELECT * FROM other_table
)
SELECT * FROM __smelt_staging_events JOIN cleaned
```

#### Inlining rules:

1. **Flat, never nested** — all CTEs hoisted to a single top-level WITH clause
2. **Namespaced** — ephemeral CTEs prefixed with `__smelt_` to prevent collisions with user CTEs
3. **Internal CTEs hoisted and renamed** — an ephemeral model's own CTEs are hoisted to top level with `__smelt_{model}__{cte}` naming, and references within the ephemeral body are rewritten to match
4. **Topologically ordered** — dependencies before dependents
5. **Deduplicated** — each ephemeral model appears once regardless of how many models reference it
6. **Ephemeral refs resolve to namespaced CTE names** — `smelt.ref('staging_events')` → `__smelt_staging_events`

#### Edge cases:

| Case | Handling |
|------|----------|
| Transitive deps (A→B→C, all ephemeral) | Hoisted in order: C, B, A |
| Diamond deps (X and Y both ref Z) | Z appears once |
| Mixed refs (ephemeral + non-ephemeral) | Ephemeral → CTE, non-ephemeral → `schema.model` |
| Existing WITH clause in downstream | Ephemeral CTEs prepended before user's CTEs |
| Ephemeral with internal CTEs | Internal CTEs hoisted with `__smelt_model__cte` naming |
| Recursive CTEs in downstream | Ephemeral CTEs (non-recursive) prepended, user's RECURSIVE preserved |
| CTE name collisions | Impossible due to `__smelt_` prefix |

### Implementation steps

**Step 2a: Printer** — Add `ephemeral_models` set to `PrintContext`, emit CTE name (no schema prefix) for ephemeral refs.

**Step 2b: EphemeralResolver** — New struct in compiler that collects transitive ephemeral deps, hoists internal CTEs with namespaced names, topologically orders, deduplicates.

**Step 2c: Execution loop** — Pre-compile ephemeral models, pass resolver to compiler, skip ephemeral models during execution.

**Step 2d: Graph** — Ephemeral models stay in DAG for ordering. Warn if ephemeral model has no consumers.

**Step 2e: Validation** — Warn if user CTE starts with `__smelt_`. Validate model names don't contain `__`.

**Step 2f: Selector filtering** — Error if user tries to `--select` an ephemeral model directly.

---

## Phase 3: Materialized View Support

### Backend trait (`smelt-backend/src/lib.rs`)
- Add `create_materialized_view_as()` and `drop_materialized_view_if_exists()` with default fallbacks
- Add `MaterializedView` arm to `execute_model()` — use MV methods if supported, else fall back to table

### Validation
- `materialized_view` + `incremental` → warn (MVs are refreshed atomically)

---

## Completed (March 26, 2026)

- Phase 1: Enum expansion + validation ✅
- Phase 2: EphemeralResolver + compile_with_ephemerals + printer changes ✅
- Phase 3: MaterializedView backend trait methods + fallback ✅

---

## Remaining Work

### Integration: Wire ephemeral models into execution loop

**File: `crates/smelt-cli/src/main.rs`**

1. Before the main execution loop, identify all ephemeral models from the execution order
2. Compile their raw SQL into `Vec<(String, String)>` (name, sql) in topological order
3. Build `EphemeralResolver::new(...)` with the collected models
4. When compiling non-ephemeral models, call `compiler.compile_with_ephemerals(model, schema, &resolver)` instead of `compiler.compile(model, schema)`
5. Skip ephemeral models in the execution loop body (print info message, continue)

### Integration: Selector filtering for ephemeral models

**File: `crates/smelt-cli/src/main.rs` or `crates/smelt-cli/src/selector.rs`**

- When `--select` targets an ephemeral model directly, emit a clear error: "Cannot run ephemeral model 'X' directly — ephemeral models are inlined as CTEs into downstream models."
- Ephemeral models selected as upstream deps (via `+model`) should be included for compilation but skipped during execution.

### Integration: Graph validation

**File: `crates/smelt-cli/src/graph.rs`**

- After building the dependency graph, warn if an ephemeral model has no downstream consumers (it would never be inlined anywhere, making it useless).

### Integration: Validation at config load

**File: `crates/smelt-cli/src/main.rs`**

- Call `config.validate_model_configs()` after loading config and metadata, and emit errors/warnings before execution begins.

### Integration: `smelt explain` output

**File: `crates/smelt-cli/src/explain.rs`**

- Show ephemeral models in explain output with `materialization: ephemeral`
- Optionally show which downstream models will inline them

### Documentation

- Update `docs/ROADMAP.md` with materialization types completion
- Update `README.md` model configuration section with `ephemeral` and `materialized_view` examples

---

## Verification

After each phase:
```bash
cargo fmt --all
cargo clippy --all-targets
cargo test
```
