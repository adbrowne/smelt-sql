# Seeds as Data, Models as Materialization

**Date:** 2026-04-06 (revised 2026-04-07)
**Status:** Proposed

## Goal

Make seeds first-class in the type system without creating tables directly. Seeds are named CSV data referenced via `smelt.seed()`. Models are the only path to materialization. Types are inferred from CSV data for LSP support; explicit CASTs in models override when needed.

## Motivation

Today seeds bypass the type system entirely — they create tables at runtime via `read_csv_auto()` with no compile-time visibility. The LSP can't offer completions, type checking, or go-to-definition for seed columns. Meanwhile, seeds creating tables directly is a special materialization path that doesn't go through models, adding complexity.

## Design

### Core Principles

1. **Seeds are data, not tables.** `smelt.seed('users')` provides CSV data. It does not create a table.
2. **Models are the only materialization path.** If you want a table from seed data, write a model.
3. **Types are inferred from CSV** for LSP purposes (completions, hover, diagnostics).
4. **Explicit CASTs in models override** inferred types when inference is wrong.
5. **`smelt.seed()` is the reference syntax** — distinct from `smelt.ref()` and `smelt.source()`.

### How It Works

A seed is a CSV file in `seeds/`:

```
seeds/
  users.csv
  events.csv
  tpch/
    customers.csv
    orders.csv
```

Naming: subdirectories are optional namespaces.
- `seeds/users.csv` → `smelt.seed('users')`
- `seeds/tpch/customers.csv` → `smelt.seed('tpch.customers')`

No prefix is required. Subdirectories disambiguate when needed.

To materialize seed data, write a model:

```sql
-- models/staging/stg_users.sql
SELECT
    user_id,
    user_name,
    CAST(signup_date AS DATE) AS signup_date
FROM smelt.seed('users')
```

The seed's inferred types flow into the type system. Here `user_id` and `user_name` keep their inferred types; `signup_date` is explicitly cast from whatever was inferred (likely VARCHAR) to DATE.

### Type Inference

At analysis time (LSP, `smelt check`), smelt reads CSV headers and sniffs column types from data. This gives the type system enough to work with:

- **Column name completion** — LSP knows what columns exist
- **Type-aware diagnostics** — expressions using seed columns get type checking
- **Hover information** — shows inferred type and nullability

All inferred columns are treated as **nullable** by default.

Inference happens by reading the CSV (headers + sample rows). The exact mechanism (DuckDB `DESCRIBE`, custom sniffing, or Arrow CSV reader) is an implementation detail.

### LSP Integration

Once seed CSVs are discovered, the LSP can:

- **Type inference**: Add seed columns to `TypeContext` with inferred types
- **Column completion**: Suggest columns after `smelt.seed('users').`
- **Hover**: Show inferred type and nullability
- **Diagnostics**: Flag references to columns that don't exist in the CSV
- **Go-to-definition**: Jump to CSV file (column-level targeting is a stretch goal)

### Dependency Graph

Seeds participate in the dependency graph as a distinct node type (like sources). `smelt.seed()` calls create edges from model → seed. This enables:

- Validation that referenced seeds exist
- Lineage tracking through seeds
- Build ordering (though seeds don't need execution — they're just data)

### CLI Changes

| Command | Current Behavior | New Behavior |
|---------|-----------------|--------------|
| `smelt seed` | Discovers CSVs, creates tables | **Removed or repurposed** — seeds don't create tables |
| `smelt build` | Runs seeds then models | Runs models only; seeds are read inline via `smelt.seed()` |
| `smelt check` | No seed awareness | Validates seed references, infers types from CSVs |

The `smelt seed` command could be repurposed to list/inspect discovered seeds and their inferred schemas (useful for debugging), or removed entirely.

### Future: YAML Shortcut

As a future enhancement, a YAML file could auto-generate staging models from seeds — reducing boilerplate for projects with many reference CSVs:

```yaml
# seeds/schema.yml (future)
seeds:
  users:
    columns:
      - name: user_id
        type: INTEGER
      - name: signup_date
        type: DATE
```

This would generate the equivalent of `SELECT CAST(user_id AS INTEGER) AS user_id, CAST(signup_date AS DATE) AS signup_date FROM smelt.seed('users')` — syntactic sugar, not a parallel type system. This is explicitly deferred and not part of this plan.

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Seeds create tables? | No | Models are the single materialization path |
| Reference syntax | `smelt.seed('name')` | Explicit, unambiguous in dependency graph |
| Naming | Subdirectory = optional namespace | `users` or `tpch.customers` — no mandatory prefix |
| Type source | Inferred from CSV data | Zero-friction; no YAML to maintain |
| Nullability | All nullable by default | Safe default; model CASTs can tighten |
| Type overrides | Explicit CASTs in model SQL | Types declared in the language users already write |

## Phases

### Phase 1: Parser and AST Support

- [ ] Add `smelt.seed()` to the lexer/parser (parallel to `smelt.source()`)
- [ ] Define `SeedCall` AST node (parallel to `SourceCall`)
- [ ] Extract seed name and optional namespace from arguments
- [ ] Parser error recovery for malformed `smelt.seed()` calls
- [ ] Tests: parsing valid/invalid seed calls, namespace extraction

### Phase 2: Seed Discovery and Type Inference

- [ ] Implement seed discovery from configured `seed_paths` (adapt existing `discover_seeds()`)
- [ ] Implement CSV type inference (read headers + sniff types from data)
- [ ] Define `SeedSchema` struct in `smelt-core` (name, namespace, columns with inferred types)
- [ ] Wire seed discovery into Salsa query layer (`seeds_config()` query)
- [ ] Populate `TypeContext` with seed columns from inferred schemas
- [ ] Tests: discovery with flat/nested directories, type inference accuracy

### Phase 3: LSP Integration

- [ ] Type inference works for expressions referencing seed columns
- [ ] Column completion for `smelt.seed()` references
- [ ] Hover shows inferred column types
- [ ] Diagnostics for undefined seed references and missing columns
- [ ] Go-to-definition navigates to CSV file
- [ ] Tests: completions, diagnostics, hover for seed columns

### Phase 4: Dependency Graph and Validation

- [ ] Add seeds as a node type in `DependencyGraph`
- [ ] `model_seeds()` query extracts `smelt.seed()` calls from models
- [ ] Validate that referenced seeds exist as discovered CSVs
- [ ] `smelt check` reports missing seed references
- [ ] Update example workspaces to use `smelt.seed()` + staging models
- [ ] Tests: graph validation, missing seed errors

### Phase 5: Remove Direct Table Creation

- [ ] Remove or repurpose `smelt seed` command (inspect/list instead of create tables)
- [ ] Remove seed table creation from `smelt build`
- [ ] Update documentation and examples
- [ ] Migration guide for existing users

## Key Files

Existing infrastructure to extend:

| File | What to reuse/modify |
|------|----------------------|
| `crates/smelt-parser/src/ast.rs` | `SourceCall` pattern → new `SeedCall` |
| `crates/smelt-parser/src/parser.rs` | `smelt.source()` parsing → add `smelt.seed()` |
| `crates/smelt-core/src/sources.rs` | Struct patterns for `SeedSchema` |
| `crates/smelt-types/src/lib.rs` | `TypedColumn`, `DataType` (reuse as-is) |
| `crates/smelt-db/src/lib.rs` | `sources_config()` pattern → `seeds_config()`, `type_context()` population |
| `crates/smelt-db/src/type_inference.rs` | `TypeContext` column storage and lookup (reuse as-is) |
| `crates/smelt-core/src/graph.rs` | `DependencyGraph` sources pattern → add seeds |
| `crates/smelt-cli/src/seed.rs` | `discover_seeds()` (adapt), `execute_seed()` (remove) |

## Open Questions

1. **Type inference mechanism** — Use DuckDB's `DESCRIBE` on `read_csv_auto()`, Arrow's CSV reader with type inference, or custom sniffing? Trade-off is accuracy vs dependency weight.
2. **`smelt seed` command fate** — Repurpose as `smelt seed list` / `smelt seed describe` for inspecting inferred schemas? Or remove entirely?
3. **Seed caching** — Should inferred schemas be cached to avoid re-reading CSVs on every LSP operation? If so, invalidation strategy when CSV changes?
