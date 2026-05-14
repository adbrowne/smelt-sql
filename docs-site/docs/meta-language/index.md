# Meta-Language

smelt's meta-language is a compile-time evaluation layer that lets you compute SQL fragments from data that is known at build time — lists of columns, configuration values, workspace introspection results — and splice those fragments into your models. The result is fully typed, editor-navigable SQL. No Jinja, no string substitution, no post-expansion parse errors.

## Two worlds: meta and data

Every smelt model lives in two overlapping worlds:

**Meta-world** — evaluated at compile time. Values are fragment sorts (`Expr<T>`, `TableExpr`, `OrderSpec`) and the meta types introduced by the meta-language (`List<T>`, `Lambda<T, U>`, records declared with `smelt.record`, and `Map<K, V>`). Meta values never reach the database engine; they are consumed during type-checking and codegen.

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

## Available constructs

The meta-language provides three constructs that exercise the meta/data boundary:

| Construct | Description | Documentation |
|-----------|-------------|---------------|
| `[a, b, c]` | Meta list literal | [Lists & Spread](lists.md) |
| `...xs` | Spread operator — splices a `List<T>` into a comma-separated position | [Lists & Spread](lists.md) |
| `List<T>` | Meta-only type: finite, ordered, immutable | [Lists & Spread](lists.md) |

The meta-language also provides iteration, transformation, compile-time configuration, schema reflection, structured data types, and file-based configuration loading:

| Construct | Description | Documentation |
|-----------|-------------|---------------|
| `fn x => body` | Lambda expression — inline single-argument function | [Lambdas](lambdas.md) |
| `map` | Apply a lambda to every element of a list | [Higher-Order Functions](hofs.md) |
| `filter` | Keep list elements matching a predicate | [Higher-Order Functions](hofs.md) |
| `reduce` | Fold a list into a single SQL fragment using a reducer | [Higher-Order Functions](hofs.md) |
| `\|>` | Pipe operator — left-to-right HOF chaining | [Pipe Operator](pipes.md) |
| `and_all`, `comma_sep`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat` | Contextual reducers | [Reducers](reducers.md) |
| `smelt.config.var('name')` | Compile-time variable lookup from `smelt.yml` | [Config Variables](config-vars.md) |
| `smelt.columns_of(t)` | Compile-time column list of a `TableExpr` → `List<ColumnRef>` | [Reflection](reflection.md) |
| `ColumnRef` | Closed meta record type: `name`, `type`, `is_numeric` fields | [Reflection](reflection.md) |
| `smelt.models.*`, `smelt.sources.*`, `ModelRef`, `SourceRef` | Wide workspace reflection — all models / sources by tag | [Reflection](reflection.md) |
| `smelt.record TypeName = { … }` | Named record-type declaration (workspace-scoped) | [Records](records.md) |
| `{f: T, …}` at type positions / `{f: v, …}` at value positions | Inline record types and record literals | [Records](records.md) |
| `r.field` | Record field projection (recursive; width subtyping applies) | [Records](records.md) |
| `Map<Text, V>` | Compile-time key-value map (invariant; loader-origin only) | [Maps](maps.md) |
| `m.entries()`, `m.keys()`, `m.values()`, `m.get(k)`, `m.has(k)` | Closed Map API (sorted ascending by key) | [Maps](maps.md) |
| `smelt.config.load_yaml(path, schema)` | Load a YAML file as a typed meta value | [Config Loaders](config-loaders.md) |
| `smelt.config.load_json(path, schema)` | Load a JSON file as a typed meta value | [Config Loaders](config-loaders.md) |

Quick reference for all constructs and diagnostic codes: [Reference](reference.md).

## Planned but not yet implemented

The following meta-language capabilities are planned but not yet available:

| Capability | Content |
|------------|---------|
| **Multi-model production** | One file generates N models |
| **Polish** | Multi-arg lambdas, meta ternary, parameterised reducers |
| **LSP completeness** | Rename, completion, diagnostics-with-frame-stacks across all surface |

These capabilities land incrementally. Each addition extends the [Reference](reference.md) page.
