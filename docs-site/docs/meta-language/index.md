# Meta-Language

smelt's meta-language is a compile-time evaluation layer that lets you compute SQL fragments from data that is known at build time — lists of columns, configuration values, workspace introspection results — and splice those fragments into your models. The result is fully typed, editor-navigable SQL. No Jinja, no string substitution, no post-expansion parse errors.

## Two worlds: meta and data

Every smelt model lives in two overlapping worlds:

**Meta-world** — evaluated at compile time. Values are fragment sorts (`Expr<T>`, `TableExpr`, `OrderSpec`) and the new meta types introduced by the meta-language (`List<T>`, and in later phases `Lambda<…>`, `Record<…>`, `Map<K,V>`). Meta values never reach the database engine; they are consumed during type-checking and codegen.

**Data-world** — the SQL the database engine sees. Types are the `DataType` vocabulary (`INTEGER`, `TEXT`, `BOOLEAN`, …). Data values exist at query runtime.

The two worlds meet at **splice points** — positions in your SQL where a meta value materialises into data-world syntax. `smelt.<name>(...)` calls are already splice points; the meta-language adds list literals and the spread operator as explicit user-facing splice points.

The same syntax can serve both worlds. For example, `[1, 2, 3]` is either a meta `List<Expr<INTEGER>>` or a data-world `Array<INTEGER>` literal, depending on the surrounding context. The type checker resolves the meaning; when both readings are valid, **meta wins**. You never need a sigil to mark meta code; the position tells the compiler which world you are in.

```sql
-- [name, email] is a meta List<Text> — it lives in a SELECT-list splice point.
-- After compilation the engine sees: SELECT id, name, email FROM ...
SELECT
    id,
    ...[name, email]
FROM smelt.sources.raw.users
```

For the full design rationale — alternatives considered, the framing of the meta/data boundary, worked examples — see the research document at `docs/research/20260507-typed-meta-programming.md`.

## What ships today (Phase A)

Phase A delivers the three constructs that exercise the meta/data boundary:

| Construct | Description | Documentation |
|-----------|-------------|---------------|
| `[a, b, c]` | Meta list literal | [Lists & Spread](lists.md) |
| `...xs` | Spread operator — splices a `List<T>` into a comma-separated position | [Lists & Spread](lists.md) |
| `List<T>` | Meta-only type: finite, ordered, immutable | [Lists & Spread](lists.md) |

Quick reference for all Phase A constructs and diagnostic codes: [Reference](reference.md).

## Phase coverage

| Phase | Status | Content |
|-------|--------|---------|
| **A — List literals, spread, `List<T>`** | Shipped | List literals, spread operator, four diagnostic codes, hover in editor |
| **B — HOFs, lambdas, pipe, reducers** | Planned | `map`, `filter`, `reduce`, lambda syntax `fn x => body`, pipe `\|>`, contextual reducers (`and_all`, `comma_sep`, …) |
| **C — Column reflection** | Deferred | `smelt.columns_of(t)`, `ColumnRef` meta record type |
| **D — Workspace reflection** | Deferred | `smelt.models.*`, `smelt.sources.*`, `ModelRef` |
| **E1 — Records, maps, config loaders** | Deferred | `Record<{…}>`, `Map<K,V>`, YAML/JSON/TOML loaders |
| **E2 — Multi-model production** | Deferred | One file generates N models |
| **F — Polish** | Deferred | Multi-arg lambdas, meta ternary, parameterised reducers |
| **G — LSP completeness** | Deferred | Rename, completion, diagnostics-with-frame-stacks across all surface |

Phase B and later phases land incrementally. Each phase adds to the [Reference](reference.md) page and may extend [Lists & Spread](lists.md) with new examples.
