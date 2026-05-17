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

The meta-language also provides iteration, transformation, compile-time branching, compile-time configuration, schema reflection, structured data types, and file-based configuration loading:

| Construct | Description | Documentation |
|-----------|-------------|---------------|
| `fn x => body` | Lambda expression — inline single-argument function | [Lambdas](lambdas.md) |
| `fn (a, b) => body` | Multi-parameter lambda (arity ≥ 2; parenthesised) | [Lambdas](lambdas.md) |
| `map` | Apply a lambda to every element of a list | [Higher-Order Functions](hofs.md) |
| `filter` | Keep list elements matching a predicate | [Higher-Order Functions](hofs.md) |
| `reduce` | Fold a list into a single SQL fragment using a reducer | [Higher-Order Functions](hofs.md) |
| `\|>` | Pipe operator — left-to-right HOF chaining | [Pipe Operator](pipes.md) |
| `and_all`, `comma_sep`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat` | Bare contextual reducers | [Reducers](reducers.md) |
| `concat_with(sep)` | Parameterised text-join reducer with a compile-time separator | [Reducers](reducers.md) |
| `if cond then a else b` | Meta-world ternary — compile-time Boolean branching with short-circuit evaluation | [Ternary](ternary.md) |
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

The meta-language also provides **multi-model production** — generating an entire family of models from a single compile-time expression:

| Construct | Description | Documentation |
|-----------|-------------|---------------|
| `generates: models` | Frontmatter directive marking a file as a generator | [Generator Files](generators.md) |
| `ModelDef { name, body, … }` | Built-in closed record: declares one emitted model | [Generator Files](generators.md) |

Quick reference for all constructs and diagnostic codes: [Reference](reference.md).

---

## How the pieces fit together

The constructs above compose. Here is a complete worked example that reads a YAML configuration file, generates one model per cohort, and then unions all the cohort models together in a downstream query. Every line references only constructs documented on this site.

### Step 1 — Load the config file and emit one model per cohort

```sql
-- models/cohorts.gen.sql
---
generates: models
tags: [cohort]
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text, region: Text, min_revenue: Integer }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT id, user_id, region, revenue, created_at
             FROM smelt.orders
             WHERE region = c.region AND revenue >= c.min_revenue
     })
```

`cohorts.yaml` lives at the workspace root:

```yaml
- name: us_west
  region: us-west-2
  min_revenue: 100
- name: us_east
  region: us-east-1
  min_revenue: 100
- name: eu
  region: eu-west-1
  min_revenue: 50
```

**What each construct does:**

- [`smelt.config.load_yaml('cohorts.yaml', List<{ … }>)`](config-loaders.md) — reads the file, validates each row against the inline schema, and returns a `List<{ name: Text, region: Text, min_revenue: Integer }>` as a compile-time meta value. The inline schema is the preferred form for short, single-use shapes; for reuse across files, [`smelt.record`](records.md) declares a named type.
- [`|>`](pipes.md) — pipes the list into the next call, keeping the chain readable left-to-right.
- [`map(fn c => ModelDef { … })`](hofs.md) — applies the [lambda](lambdas.md) to every cohort, converting each record into a [`ModelDef`](generators.md). The result is `List<ModelDef>`.
- [`generates: models`](generators.md) frontmatter — marks the file as a generator. smelt expands the `List<ModelDef>` and emits three models: `smelt.cohorts.us_west`, `smelt.cohorts.us_east`, `smelt.cohorts.eu`.

### Step 2 — Union all cohort models in a downstream query

There are two patterns for the downstream union. The **explicit form** lists each emitted model by name — useful for clarity in small, stable cohort sets:

```sql
-- models/all_cohorts_unioned.sql (explicit form — matches examples/per_cohort_union/)
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_west
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_east
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.eu
```

The **dynamic-discovery form** uses workspace reflection so that adding a new cohort to `cohorts.yaml` automatically emits a new model and adds it to the union — no SQL edits required:

```sql
-- models/all_cohorts_unioned.sql (dynamic-discovery form)
SELECT * FROM reduce(
    smelt.models.with_tag('cohort'),
    union_all
)
```

**What the dynamic form does:**

- [`smelt.models.with_tag('cohort')`](reflection.md) — returns `List<ModelRef>` for every model in the workspace tagged `cohort`, including all models emitted by the generator in Step 1.
- [`reduce(…, union_all)`](hofs.md) — folds the list into a single `TableExpr` using [the `union_all` reducer](reducers.md), producing one `UNION ALL` branch per cohort.

The runnable starter in `examples/per_cohort_union/` uses the explicit form for clarity. The dynamic-discovery form is the pattern to reach for when the cohort list grows or changes frequently.
