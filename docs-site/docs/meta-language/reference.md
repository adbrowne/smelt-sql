# Meta-Language Reference

Alphabetical quick reference for all meta-language constructs and diagnostic codes. Covers list literals, the spread operator, every HOF, reducer, lambda keyword, the pipe operator, the ternary expression, `smelt.config.var`, the reflection surface (`smelt.columns_of`, `ColumnRef`, identifier lift, wide reflection accessors `smelt.models.*` / `smelt.sources.*`, `ModelRef`, `SourceRef`), and generator files (`<generator>` frame, `generates: models`, `generator_file:` selector, `ModelDef`, `origin`).

For a conceptual introduction, see [Overview](index.md). For detailed explanations, see the per-construct pages: [Lists & Spread](lists.md), [Lambdas](lambdas.md), [Higher-Order Functions](hofs.md), [Pipe Operator](pipes.md), [Reducers](reducers.md), [Ternary](ternary.md), [Config Variables](config-vars.md), [Reflection](reflection.md), [Records](records.md), [Maps](maps.md), [Config Loaders](config-loaders.md), [Generator Files](generators.md).

---

## `<generator>` frame

A **generator file** is any `.gen.sql` file whose YAML frontmatter carries `generates: models`. Its body is a meta-language expression of type `List<ModelDef>`; each element is expanded into a standalone model at build time.

The `<generator>` frame is the unit of expansion: one generator file produces zero or more models, each identified by `ModelDef.name`. Generator files may drive model production from config loaders (`smelt.config.load_yaml`) or from workspace source reflection (`smelt.sources.with_tag`, `smelt.sources.all`). Model reflection (`smelt.models.*`) is forbidden inside a generator body.

See [Generator Files](generators.md) for the full specification, the `ModelDef` record type, collision rules, and the `generator_file:` CLI selector.

---

## `and_all` — Boolean AND reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Boolean>>, and_all) -> Expr<Boolean>
```

**Empty-list identity:** `TRUE`

**Example:**
```sql
-- reduce([is_active, age > 18], and_all) → is_active AND age > 18
SELECT id FROM smelt.sources.raw.users
WHERE reduce([is_active, age > 18], and_all)
```

See [Reducers — `and_all`](reducers.md#and_all) for full details.

---

## `ColumnRef` — closed meta record type for column reflection

**Kind:** closed meta-only record type; produced exclusively by `smelt.columns_of`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `name` | `Text` | Column identifier (un-quoted; case-preserved) |
| `type` | `DataType` | Column's `DataType` |
| `is_numeric` | `Boolean` | `TRUE` iff `type` is in the `Numeric` constraint set |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `ColumnRefFieldUnknown`. `ColumnRef` is not user-constructible — values originate only from `smelt.columns_of`.

**Example:**
```sql
smelt.columns_of(smelt.orders)
  |> filter(fn c => c.is_numeric)   -- c.is_numeric : Boolean
  |> map(fn c => c.name)            -- c.name : Text, lifts to identifier in splice
```

**Editor support:** hover on a `ColumnRef`-typed binding shows `ColumnRef` and the closed field list with each field's type; completion at `c.<cursor>` offers `name`, `type`, `is_numeric`.

See [Reflection — `ColumnRef`](reflection.md#columnref) for the closed-field contract, body-check vs expansion-time behaviour, and diagnostic codes.

---

## `comma_sep` — comma-separated select-items reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<T>>, comma_sep) -> SelectItems<Scalar>
```

**Empty-list identity:** empty `SelectItems` (adjacent commas elide at splice)

**Example:**
```sql
SELECT reduce([id, name, email], comma_sep) FROM smelt.sources.raw.users
-- Engine sees: SELECT id, name, email FROM ...
```

See [Reducers — `comma_sep`](reducers.md#comma_sep) for full details.

---

## `concat` — text concatenation reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Text>>, concat) -> Expr<Text>
```

**Empty-list identity:** `''` (empty string)

**Example:**
```sql
SELECT reduce(['hello', ' ', 'world'], concat)
-- Engine sees: SELECT 'hello' || ' ' || 'world'
```

See [Reducers — `concat`](reducers.md#concat) for full details.

---

## `concat_with(sep)` — parameterised text-join reducer

**Kind:** parameterised contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Text>>, concat_with(sep: Text)) -> Expr<Text>
```

**Parameter:** `sep` — a compile-time `Text` separator (string literal or `smelt.config.var(...)` result).

**Empty-list identity:** `''` (independent of `sep`)

**Example:**
```sql
-- reduce(['alpha', 'beta', 'gamma'], concat_with(' OR '))
-- → 'alpha' || ' OR ' || 'beta' || ' OR ' || 'gamma'
SELECT reduce(['alpha', 'beta', 'gamma'], concat_with(' OR '))
```

See [Reducers — `concat_with`](reducers.md#concat_with) for full details.

---

## `filter` — HOF: keep list elements matching a predicate

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
filter(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>
```

**Example:**
```sql
-- Keep only positive numbers
SELECT filter([1, -2, 3], fn c => c > 0)
```

**Editor support:** hover shows `List<T>` with `T` from the input element type.

See [Higher-Order Functions — `filter`](hofs.md#filter) for full details and diagnostic codes.

---

## `fn` — lambda keyword

**Kind:** reserved keyword; introduces a lambda expression.

**Syntax:**
```
fn IDENT => EXPR                          -- single-parameter form
fn ( IDENT_1 , IDENT_2 , … ) => EXPR     -- multi-parameter form (k ≥ 1)
```

Only valid as a positional argument to a HOF (`map`, `filter`). A lambda cannot be assigned to a name, stored in a list, or passed as a named argument.

The single-parameter form and the parenthesised single-parameter form `fn (x) => body` are equivalent. The multi-parameter form `fn (a, b) => body` declares a lambda of arity ≥ 2; the parameter list is parenthesised. All parameters within one lambda must have distinct names.

**Example — single parameter:**
```sql
-- fn c => c * 2 doubles each element of the list
SELECT map([1, 2, 3], fn c => c * 2)
```

**Example — parenthesised single parameter:**
```sql
-- fn (c) => c * 2 is equivalent to fn c => c * 2
SELECT map([1, 2, 3], fn (c) => c * 2)
```

**Editor support:** hover on a parameter inside the body shows its bound type; goto-definition resolves to the parameter's binding occurrence in the `fn` head; completion inside the body offers the bound parameters first.

See [Lambdas](lambdas.md) for full details, scoping rules, and diagnostic codes.

---

## `generates: models` — generator file frontmatter directive

**Kind:** YAML frontmatter key; marks the file as a generator file.

**Syntax:** `generates: models` (the only valid value in v1).

A generator file's body is a meta-language expression of type `List<ModelDef>`. Each emitted `ModelDef` value with `name: 'n'` becomes a model at smelt path `smelt.<dir>.<stem>.<n>`.

**Editor support:** hover on `generates: models` shows `List<ModelDef>` and (when statically resolvable) the number of emitted models; completion at `generates: <cursor>` offers exactly `models`.

See [Generator Files](generators.md) for the full specification.

---

## `generator_file:` — CLI selector for generator-emitted models

**Kind:** CLI / catalog selector key; scopes commands to models emitted by a specific generator file.

**Syntax:**
```
generator_file: models/path/to/file.gen.sql
```

Pass `generator_file:` as a selector to `smelt build`, `smelt explain`, or `smelt test` to target only the models produced by the named generator. The path is workspace-relative and uses `/` as the separator on all platforms.

**Example:**
```
smelt build generator_file:models/cohorts.gen.sql
```

See [Generator Files — CLI and catalog integration](generators.md) for full details and interaction with tag-based selectors.

---

## `if … then … else …` — meta-world ternary expression

**Kind:** meta-world compile-time expression; not a statement.

**Syntax:**
```
if COND then THEN_EXPR else ELSE_EXPR
```

**Pseudo-signature:** `(Boolean, T, T) -> T` — `COND` must be `Boolean`; `THEN_EXPR` and `ELSE_EXPR` must unify under LUB; the ternary's type is the LUB.

**Keywords:** `if`, `then`, `else` are reserved at the meta-namespace level.

**Precedence:** lower than `|>` (pipe). **Associativity:** right-associative chaining — `else if c then x else y` nests as `else (if c then x else y)`.

**Short-circuit:** exactly one of `THEN_EXPR` / `ELSE_EXPR` is evaluated at compile time. Evaluation-time diagnostics (`MapGetMissingKey`, `ConfigVarNotFound`) on the unreached branch are suppressed; type-checking diagnostics still fire on both branches.

**Example:**
```sql
-- Resolves to 'strict' at compile time; engine sees: SELECT 'strict'
SELECT if true then 'strict' else 'permissive'
```

**Defaulting pattern:**
```sql
-- Guard a map.get with m.has to avoid MapGetMissingKey on the false branch
SELECT if m.has('env') then m.get('env') else 'production'
```

**Editor support:** hover on `if` shows the full ternary signature with resolved types; hover on `then`/`else` shows the corresponding branch's type; completion offers `if` as a snippet.

See [Ternary](ternary.md) for full details, precedence rules, and diagnostic codes.

---

## `intersect_all` — table INTERSECT ALL reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<TableExpr>, intersect_all) -> TableExpr
```

**Empty-list identity:** none — `ReducerEmptyNoIdentity` on an empty list.

**Example:**
```sql
SELECT * FROM reduce(
    [smelt.ref('active_users'), smelt.ref('premium_users')],
    intersect_all
)
```

See [Reducers — `intersect_all`](reducers.md#intersect_all) for full details.

---

## `List<T>` — meta list type

**Kind:** meta-only type; never appears in data-world SQL.

**Definition:** a finite, ordered, immutable sequence of elements of type `T`. Length is fixed at construction. `T` is a fragment sort (`Expr<U>`, `OrderSpec`, …) or a data-type lifted as a meta literal (`Text`, `Integer`, …).

**Covariance:** `List<S> <: List<T>` whenever `S <: T`. Sound because lists are immutable.

**Construction:** list literals `[a, b, c]`; HOFs `map`, `filter`.

**Hover:** hovering over a list literal shows `List<T>` with `T` resolved to the inferred element type, e.g. `List<Expr<INTEGER>>`.

**Example:**
```sql
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
--     ^^^^^^^^^^
--     List<Expr<INTEGER>> — hover shows this type in the editor
```

See [Lists & Spread — The `List<T>` type](lists.md#the-listt-type) for the covariance rule and subtyping details.

---

## `m.entries` — Map entries method

**Kind:** Map API method; returns `List<{key: K, value: V}>`.

**Signature:**
```
m.entries() -> List<{key: K, value: V}>
```

Returns all key-value pairs sorted ascending by key. Takes no arguments; any argument emits `MapApiUnexpectedArgument`.

See [Maps — Map API](maps.md#map-api) for full details.

---

## `m.get` — Map key lookup

**Kind:** Map API method; returns `V`.

**Signature:**
```
m.get(k: K) -> V
```

Returns the value bound to `k`. A string-literal `k` statically absent from `m` emits `MapGetMissingKey`. Takes exactly one positional argument.

See [Maps — `m.get` missing-key behaviour](maps.md#mget-missing-key-behaviour) for the statically-known vs deferred resolution rules.

---

## `m.has` — Map key presence check

**Kind:** Map API method; returns `Boolean`.

**Signature:**
```
m.has(k: K) -> Boolean
```

Returns `TRUE` iff `m` contains a binding for `k`. Takes exactly one positional argument.

See [Maps — Map API](maps.md#map-api) for full details.

---

## `m.keys` — Map keys

**Kind:** Map API method; returns `List<K>`.

**Signature:**
```
m.keys() -> List<K>
```

Returns the key set sorted ascending. Takes no arguments; any argument emits `MapApiUnexpectedArgument`.

See [Maps — Map API](maps.md#map-api) for full details.

---

## `m.values` — Map values

**Kind:** Map API method; returns `List<V>`.

**Signature:**
```
m.values() -> List<V>
```

Returns values ordered by their corresponding keys' ascending sort. Takes no arguments; any argument emits `MapApiUnexpectedArgument`.

See [Maps — Map API](maps.md#map-api) for full details.

---

## `map` — HOF: apply a lambda to every element

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
map(xs: List<T>, f: Lambda<T, U>) -> List<U>
```

**Example:**
```sql
-- Double every element
SELECT map([1, 2, 3], fn c => c * 2)
```

**Editor support:** hover shows `List<U>` with `U` from the lambda body's inferred type.

See [Higher-Order Functions — `map`](hofs.md#map) for full details and diagnostic codes.

---

## `Map<K, V>` — meta map type

**Kind:** meta-only type; never appears in data-world SQL.

**Definition:** a finite, immutable key-value collection. In v1 `K` must be `Text`; `V` is any meta-language type. Invariant in both `K` and `V`. Produced exclusively by the config-loader family.

**Iteration order:** byte-lexicographic ascending by key for `entries()`, `keys()`, and `values()`.

**No literal syntax in v1** — maps originate from `smelt.config.load_yaml` / `smelt.config.load_json` with a `Map<Text, S>` schema.

**Example:**
```sql
-- Load a YAML mapping and iterate over entries
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>)
```

See [Maps](maps.md) for the full API, missing-key behaviour, and diagnostic codes.

---

## Meta-`Text`-as-identifier lift positions {#meta-text-as-identifier-lift-positions}

When a meta-`Text` value is spliced into a position where the SQL grammar expects an unquoted **identifier**, smelt lifts that value to the identifier. The lift applies in exactly four positions:

| Position | Example |
|---|---|
| Column-reference position inside an expression | `COALESCE(c.name, 0)` — `c.name` lifts to a column identifier |
| `AS` alias of a SELECT item | `SUM(amount) AS c.name` — `c.name` lifts to the output alias |
| `ORDER BY` column reference | `ORDER BY c.name` — `c.name` lifts to a sort key |
| `GROUP BY` column reference | `GROUP BY c.name` — `c.name` lifts to a grouping key |

In all other positions a meta-`Text` retains its string-value meaning. The lifted identifier is validated against the surrounding splice context using the standard scoping rule; an unrecognised column name emits `UnknownColumn`.

The lift applies **only to compile-time meta-`Text` values**, not to runtime `Expr<Text>` values.

See [Reflection — Meta-`Text`-as-identifier lift](reflection.md#meta-text-as-identifier-lift) for full details and examples.

---

## `ModelDef` — built-in closed record type for generator files

**Kind:** closed meta-only record type; user-constructible only inside a generator file body (`generates: models`).

**Fields (in declaration order):**

| Field | Type | Required | Default |
|-------|------|----------|---------|
| `name` | `Text` | Yes | — |
| `body` | `TableExpr` | Yes | — |
| `materialization` | `Text` | No | `"view"` |
| `tags` | `List<Text>` | No | `[]` |
| `description` | `Text` | No | `""` |

Constructing a `ModelDef` literal outside a generator file body emits `ModelDefOutsideGeneratorFile`.

**Example:**
```sql
---
generates: models
---
[
  ModelDef { name: 'orders',  body: SELECT * FROM smelt.sources.raw.orders },
  ModelDef { name: 'users',   body: SELECT * FROM smelt.sources.raw.users }
]
```

**Editor support:** hover on the opening `{` of a `ModelDef` literal shows the emitted smelt path (when `name` is statically known); completion at `ModelDef { <cursor>` offers the five fields, required fields first.

See [Generator Files](generators.md) for the full multi-model production surface.

---

## `ModelRef` — closed meta record type for model reflection

**Kind:** closed meta-only record type; produced by `smelt.models.with_tag` and `smelt.models.all`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative path (e.g. `models/orders.sql`) |
| `name` | `Text` | Short model name (stem, e.g. `orders`) |
| `tags` | `List<Text>` | Merged tag set (smelt.yml first, then frontmatter) |
| `columns` | `List<ColumnRef>` | Column list — equivalent to `smelt.columns_of(m)` |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `ModelRefFieldUnknown`. `ModelRef` is not user-constructible — values originate only from `smelt.models.*` accessors.

**Subtyping:** `ModelRef <: TableExpr`. Pass a `ModelRef` anywhere a `TableExpr` is required (e.g. `smelt.columns_of`, `reduce(..., union_all)`) without explicit projection.

**Example:**
```sql
-- Collect the name of every model tagged 'cohort'
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)

-- m.columns is equivalent to smelt.columns_of(m)
SELECT map(smelt.models.with_tag('cohort'), fn m => m.columns)
```

**Editor support:** hover on a `ModelRef`-typed binding shows `ModelRef` and the closed field list with each field's type; completion at `m.<cursor>` offers `path`, `name`, `tags`, `columns`.

See [Reflection — `ModelRef`](reflection.md#modelref) for the closed-field contract, subtyping rules, and diagnostic codes.

---

## `or_any` — Boolean OR reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Boolean>>, or_any) -> Expr<Boolean>
```

**Empty-list identity:** `FALSE`

**Example:**
```sql
-- reduce([is_admin, is_moderator], or_any) → is_admin OR is_moderator
SELECT id FROM smelt.sources.raw.users
WHERE reduce([is_admin, is_moderator], or_any)
```

See [Reducers — `or_any`](reducers.md#or_any) for full details.

---

## `origin` — model provenance field in CLI and catalog output

**Kind:** metadata field; present in `smelt explain --json` output and in catalog markdown for every model, whether hand-authored or generator-emitted.

For hand-authored models, `origin` carries `{"kind": "cli"}` (when built via the CLI) or `{"kind": "catalog"}` (when queried from the catalog). For generator-emitted models, `origin` additionally records `{"generator_file": "models/path/to/file.gen.sql", "model_def_name": "name"}` so that downstream tools can trace each model back to its producing generator.

See [Generator Files](generators.md) for full details on the `origin` field and how it appears in `smelt explain --json` and catalog output.

---

## `plus_chain` — numeric addition reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Numeric>>, plus_chain) -> Expr<Numeric>
```

**Empty-list identity:** `0` cast to the LUB element type.

**Example:**
```sql
SELECT reduce([1, 2, 3], plus_chain)
-- Engine sees: SELECT 1 + 2 + 3
```

See [Reducers — `plus_chain`](reducers.md#plus_chain) for full details.

---

## `reduce` — HOF: fold a list into a single fragment

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
reduce(xs: List<T>, r) -> OutputSort
```

where `r` is a bare reducer identifier from the closed registry and `OutputSort` is the reducer's declared output sort.

**Example:**
```sql
SELECT reduce([true, false, true], and_all)
-- Engine sees: SELECT true AND false AND true
```

**Editor support:** hover on the reducer name shows its input element type, output sort, and empty-list identity.

See [Higher-Order Functions — `reduce`](hofs.md#reduce) and [Reducers](reducers.md) for full details.

---

## `smelt.columns_of` — compile-time column list accessor

**Kind:** built-in meta-only accessor; returns `List<ColumnRef>`.

**Signature:**
```
smelt.columns_of(t: TableExpr) -> List<ColumnRef>
```

Returns the column list of a `TableExpr`-valued argument as a `List<ColumnRef>`, preserving declared column order. Must be called with exactly one positional argument; named arguments emit `ColumnsOfNamedArgument`. The argument must be `TableExpr`-typed; mismatches emit `ColumnsOfRequiresTableExpr`.

**Example:**
```sql
smelt.columns_of(smelt.orders)
  |> filter(fn c => c.is_numeric)
  |> map(fn c => COALESCE(c.name, 0))
```

**Editor support:** hover on `smelt.columns_of(t)` shows `List<ColumnRef>` and, when `t`'s schema is statically resolvable, the resolved column count plus the first five column names; completion at the argument position offers in-scope `TableExpr`-valued names.

See [Reflection](reflection.md) for the full surface, body-check vs expansion-time semantics, and worked example.

---

## `smelt.config.load_json` — JSON file loader

**Kind:** built-in compile-time loader; returns the schema type.

**Signature:**
```
smelt.config.load_json(path: Text, schema: Schema) -> Schema
```

Loads the JSON file at `path` (workspace-relative string literal) and validates its contents against `schema`. `Schema` must be an inline record type, a named `smelt.record` name, `List<S>`, or `Map<Text, S>`. Supports per-target overlays via `<basename>.<target>.json` sibling files.

**Example:**
```sql
SELECT smelt.config.load_json('configs/settings.json', {debug: Boolean, timeout: Integer})
```

See [Config Loaders](config-loaders.md) for path rules, schema authoring, per-target overlay, and diagnostic codes.

---

## `smelt.config.load_toml` — reserved loader name

**Kind:** reserved built-in name; always emits `ConfigLoaderTomlNotYetSupported`.

`smelt.config.load_toml` is reserved for a future TOML loader. Using it at any call site emits `ConfigLoaderTomlNotYetSupported`. Convert your TOML config to YAML or JSON and use `load_yaml` / `load_json` instead.

See [Config Loaders — Diagnostic codes](config-loaders.md#diagnostic-codes) for the `ConfigLoaderTomlNotYetSupported` entry.

---

## `smelt.config.load_yaml` — YAML file loader

**Kind:** built-in compile-time loader; returns the schema type.

**Signature:**
```
smelt.config.load_yaml(path: Text, schema: Schema) -> Schema
```

Loads the YAML file at `path` (workspace-relative string literal) and validates its contents against `schema`. `Schema` must be an inline record type, a named `smelt.record` name, `List<S>`, or `Map<Text, S>`. Supports per-target overlays via `<basename>.<target>.yaml` sibling files.

**Example:**
```sql
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

SELECT smelt.config.load_yaml('configs/cohorts.yaml', List<Cohort>)
```

**Editor support:** hover shows the resolved file path and entry count; goto-definition on the path literal resolves to the loaded file; completion at the path argument offers workspace YAML files.

See [Config Loaders](config-loaders.md) for path rules, schema authoring, per-target overlay, and diagnostic codes.

---

## `smelt.config.var` — compile-time variable lookup

**Kind:** built-in compile-time function; returns `Text`.

**Signature:**
```
smelt.config.var(name: Text) -> Text
```

Reads `name` from the `vars:` block of `smelt.yml`. The argument must be a string literal.

**Example:**
```sql
SELECT smelt.config.var('region')
-- Resolves to: SELECT 'us-west-2'  (when smelt.yml declares vars: {region: us-west-2})
```

**Editor support:** hover shows `Text` and the variable's resolved value; goto-definition resolves to the `vars.name:` line in `smelt.yml`.

See [Config Variables](config-vars.md) for YAML scalar coercion rules, diagnostic codes, and worked examples.

---

## `smelt.models.all` — all workspace models

**Kind:** compile-time workspace accessor; returns `List<ModelRef>`.

**Signature:**
```
smelt.models.all() -> List<ModelRef>
```

Returns every model in the workspace, sorted by workspace-relative path (byte-lexicographic, `/` separator). Takes no arguments; any argument emits `WideReflectionUnexpectedArgument`.

**Example:**
```sql
-- All model paths in the workspace
SELECT map(smelt.models.all(), fn m => m.path)
```

**Editor support:** hover on `smelt.models.all()` shows `List<ModelRef>` and the resolved model count; completion offers `smelt.models.all` in `smelt.models.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.models.with_tag` — workspace models filtered by tag

**Kind:** compile-time workspace accessor; returns `List<ModelRef>`.

**Signature:**
```
smelt.models.with_tag(tag: Text) -> List<ModelRef>
```

Returns all workspace models whose tag set contains `tag`, sorted by workspace-relative path. Argument must be a single positional compile-time string literal. Named arguments emit `WithTagNamedArgument`; non-literal arguments emit `WithTagRequiresText`.

**Example:**
```sql
-- All models tagged 'cohort', sorted by path
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)
```

**Editor support:** hover on `smelt.models.with_tag(...)` shows `List<ModelRef>` and the resolved model count for the given tag; completion offers `smelt.models.with_tag` in `smelt.models.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.record` — named record-type declaration

**Kind:** top-level workspace-scoped declaration; introduces a named meta record type.

**Syntax:**
```
smelt.record TypeName = { field1: Type1, field2: Type2, … }
```

Declares a named record type at workspace scope. The name must be unique workspace-wide; a second declaration of the same name emits `SmeltRecordRedefinition`. Field types may be scalar `DataType` literals, `List<T>`, `Map<Text, V>`, inline record types `{…}`, or previously declared `smelt.record` names. Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) are not valid field types and emit `RecordFieldTypeForbidden`.

After declaration, `TypeName` is usable as a type in any type-annotation position — loader schema arguments, field types of other records, `Map<Text, TypeName>` value types.

**Example:**
```sql
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

-- Use as a loader schema
SELECT smelt.config.load_yaml('configs/cohorts.yaml', List<Cohort>)
```

**Editor support:** hover on a `smelt.record` name shows the full field list; goto-definition on any usage resolves to the declaration site; completion at record-literal and field-projection positions offers the declared field set.

See [Records](records.md) for inline record types, record literals, field projection, width subtyping, and diagnostic codes.

---

## `smelt.sources.all` — all workspace sources

**Kind:** compile-time workspace accessor; returns `List<SourceRef>`.

**Signature:**
```
smelt.sources.all() -> List<SourceRef>
```

Returns every declared source in the workspace, sorted by workspace-relative path (byte-lexicographic, `/` separator). Takes no arguments; any argument emits `WideReflectionUnexpectedArgument`.

**Example:**
```sql
-- All source paths in the workspace
SELECT map(smelt.sources.all(), fn s => s.path)
```

**Editor support:** hover on `smelt.sources.all()` shows `List<SourceRef>` and the resolved source count; completion offers `smelt.sources.all` in `smelt.sources.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.sources.with_tag` — workspace sources filtered by tag

**Kind:** compile-time workspace accessor; returns `List<SourceRef>`.

**Signature:**
```
smelt.sources.with_tag(tag: Text) -> List<SourceRef>
```

Returns all workspace sources whose tag set contains `tag`, sorted by workspace-relative path. Argument must be a single positional compile-time string literal. Named arguments emit `WithTagNamedArgument`; non-literal arguments emit `WithTagRequiresText`.

**Example:**
```sql
-- All sources tagged 'audit'
SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)
```

**Editor support:** hover on `smelt.sources.with_tag(...)` shows `List<SourceRef>` and the resolved source count for the given tag; completion offers `smelt.sources.with_tag` in `smelt.sources.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `SourceRef` — closed meta record type for source reflection

**Kind:** closed meta-only record type; produced by `smelt.sources.with_tag` and `smelt.sources.all`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative path (e.g. `models/sources/raw/orders.yml`) |
| `name` | `Text` | Short source name (e.g. `orders`) |
| `tags` | `List<Text>` | Merged tag set (smelt.yml first, then frontmatter) |
| `columns` | `List<ColumnRef>` | Column list — equivalent to `smelt.columns_of(s)` |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `SourceRefFieldUnknown`. `SourceRef` is not user-constructible — values originate only from `smelt.sources.*` accessors.

**Subtyping:** `SourceRef <: TableExpr`. Pass a `SourceRef` anywhere a `TableExpr` is required without explicit projection.

**Example:**
```sql
-- Collect the name of every source tagged 'audit'
SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)
```

**Editor support:** hover on a `SourceRef`-typed binding shows `SourceRef` and the closed field list with each field's type; completion at `s.<cursor>` offers `path`, `name`, `tags`, `columns`.

See [Reflection — `SourceRef`](reflection.md#sourceref) for the closed-field contract, subtyping rules, and diagnostic codes.

---

## `union_all` — table UNION ALL reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<TableExpr>, union_all) -> TableExpr
```

**Empty-list identity:** none — `ReducerEmptyNoIdentity` on an empty list.

**Example:**
```sql
SELECT * FROM reduce(
    [smelt.ref('orders_2024'), smelt.ref('orders_2025')],
    union_all
)
```

See [Reducers — `union_all`](reducers.md#union_all) for full details.

---

## `...xs` — spread operator

**Type:** consumes `List<T>`; materialises `T` elements into the surrounding comma-separated grammar position.

**Syntax:**
```
...expr
```

where `expr` evaluates to a `List<T>`.

**Valid positions:** SELECT lists.

**Forbidden positions:** WHERE clauses, FROM clauses without an explicit reducer, boolean-composition contexts (`AND`/`OR`), named-argument positions (`name => value`). Each forbidden use emits `MetaSpreadInForbiddenPosition`.

**Empty-list behaviour:** `...[]` elides itself and adjacent commas silently.

**Hover:** hovering over `...xs` in the editor shows the type of the source list, e.g. `List<Expr<INTEGER>>`.

**Example:**
```sql
-- Spread two column references into SELECT
SELECT id, ...[name, email] FROM smelt.sources.raw.users
-- Engine sees: SELECT id, name, email FROM ...
```

See [Lists & Spread — Spread operator](lists.md#spread-operator-xs) for full details and forbidden-position examples.

---

## `.field` — record field projection

**Kind:** meta-world infix operator; navigates a record-typed value to a named field.

**Syntax:**
```
expr.fieldname
```

where `expr` evaluates to a record type (named or inline) and `fieldname` is a declared field of that type.

**Recursive projection:** `r.outer.inner` projects `outer` then `inner`. Each step type-checks independently. Projecting through a non-record-typed intermediate emits `RecordFieldNotProjectable`.

**Width subtyping:** projection on a value of type `{a: T, b: U}` succeeds for both `r.a` and `r.b`. The static type governs the closed field set; projecting a field not declared on the static type emits `RecordFieldUnknown` even if the runtime value carries that field.

**Example:**
```sql
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

-- r.name : Text, r.threshold : Integer
SELECT map(cohorts, fn r => r.name || ': ' || CAST(r.threshold AS TEXT))
```

**Editor support:** completion at `r.<cursor>` offers the closed field list; hover shows the projected field's declared type; goto-definition resolves to the field in the `smelt.record` declaration.

See [Records — Field projection](records.md#field-projection) for the full projection contract and diagnostic codes.

---

## `[…]` — list literal

**Type:** `List<T>` where `T` is the LUB of element types.

**Syntax:**
```
[ expr, expr, … ]          -- one or more elements
[ expr, expr, …, ]         -- trailing comma allowed
[ expr ]                   -- singleton
[]                         -- empty (requires inferable target sort)
```

**Disambiguation:** the same `[…]` token sequence may resolve to a meta `List<T>` or a data-world `Array<U>` depending on the surrounding context. When both readings are valid, meta wins.

**Example:**
```sql
-- List<Expr<INTEGER>> spliced into a SELECT list
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
-- Engine sees: SELECT 1, 2, 3 FROM ...
```

See [Lists & Spread — List literal syntax](lists.md#list-literal-syntax-a-b-c) for full details.

---

## `{…}` — record literal / inline record type

**Dual role:** `{…}` is used at two distinct positions in the meta-language:

1. **At a type-annotation position** — an inline (anonymous) record type: `{field1: Type1, field2: Type2, …}`. Inline record types are structurally typed; two records with the same field set are the same type. Used as loader schemas and in `Map<Text, {…}>` type expressions.

2. **At a value position** — a record literal: `{field1: value1, field2: value2, …}`. Bidirectionally type-checked against the surrounding target type; emits `RecordFieldMissing`, `RecordFieldUnknown`, `RecordFieldDuplicate`, or `RecordFieldTypeMismatch` on violations.

**Width subtyping applies to both:** `{a: T, b: U}` is a subtype of `{a: T}` (the wider record is the subtype).

**Example (type position):**
```sql
-- {name: Text, threshold: Integer} is an inline record schema
SELECT smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, threshold: Integer}>)
```

**Example (value position):**
```sql
-- {name: 'eu', threshold: 50} is a record literal
smelt.record Cohort = { name: Text, threshold: Integer }
SELECT {name: 'eu', threshold: 50}
```

See [Records](records.md) for inline record types, record literals, width subtyping, and diagnostic codes.

---

## `|>` — pipe operator

**Kind:** meta-world binary operator; purely syntactic sugar desugared before type-checking.

**Semantics:**
```
LHS |> f(args...)   ≡   f(LHS, args...)
```

Left-associative, lowest meta-language precedence. RHS must be a call expression.

**Example:**
```sql
-- examples/meta_hofs/models/pipe_rewrite.sql
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- desugars to: map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
```

**Editor support:** hover shows the result type of the equivalent un-piped call.

See [Pipe Operator](pipes.md) for full details and diagnostic codes.

---

## Diagnostic codes

Alphabetical across the whole meta-language surface.

### `ColumnRefFieldUnknown`

**When:** Field access on a `ColumnRef` value uses an identifier that is not one of the three declared fields (`name`, `type`, `is_numeric`).

**Message:** `ColumnRef has no field {name}; expected one of: name, type, is_numeric`

**Fix:** use `c.name` (Text), `c.type` (DataType), or `c.is_numeric` (Boolean). Any other field requires a spec extension.

See [Reflection — `ColumnRefFieldUnknown`](reflection.md#columnrefieldunknown).

---

### `ColumnsOfNamedArgument`

**When:** `smelt.columns_of` is called with a named argument instead of a positional one.

**Message:** `smelt.columns_of takes one positional argument; named arguments are not supported`

**Fix:** pass the `TableExpr` positionally: `smelt.columns_of(smelt.orders)`.

See [Reflection — `ColumnsOfNamedArgument`](reflection.md#columnsofnamedargument).

---

### `ColumnsOfRequiresTableExpr`

**When:** The argument to `smelt.columns_of` synthesises to a type not assignable to `TableExpr`.

**Message:** `smelt.columns_of expects TableExpr; found {actual}`

**Fix:** pass a `smelt.<path>` reference to a model, source, or seed, or a `TableExpr` parameter of the enclosing `smelt.define` function.

See [Reflection — `ColumnsOfRequiresTableExpr`](reflection.md#columnsofRequirestableexpr).

---

### `ColumnsOfUnresolvableSchema`

**When:** At expansion time, `smelt.columns_of(t)` cannot resolve the schema for `t` (for example because an upstream model has an unknown schema).

**Message:** `cannot resolve column list for {t}; upstream schema is unknown`

**Fix:** ensure the upstream model, source, or seed has a fully declared schema and compiles cleanly. This diagnostic suppresses further errors from the surrounding HOF call.

See [Reflection — `ColumnsOfUnresolvableSchema`](reflection.md#columnsofunresolvableschema).

---

### `ConfigLoaderDuplicateMapKey`

**When:** A `Map<Text, S>`-shaped YAML/JSON file contains the same key twice.

**Message:** `duplicate map key '{key}' at {row}; earlier appearance at {first_row}`

**Fix:** remove the duplicate key from the source file, keeping the intended value.

See [Config Loaders — Diagnostic codes](config-loaders.md#diagnostic-codes).

---

### `ConfigLoaderFileNotFound`

**When:** The resolved workspace-relative file does not exist.

**Message:** `loader file '{path}' not found in workspace`

**Fix:** create the missing file or correct the path literal.

See [Config Loaders — `ConfigLoaderFileNotFound`](config-loaders.md#configloaderfilenotfound).

---

### `ConfigLoaderNullCoercion` (warning)

**When:** A YAML `null` scalar coerces to empty `Text` at a schema field declared `Text`.

**Message:** `null value at {row} coerced to empty string; declare a default in the source file`

**Fix:** replace the null with an explicit empty string or a meaningful default in the YAML file.

See [Config Loaders — `ConfigLoaderNullCoercion`](config-loaders.md#configloadernullcoercion-warning).

---

### `ConfigLoaderParseError`

**When:** The loaded file is not valid YAML or JSON.

**Message:** `failed to parse {format} file '{path}': {parser_error}`

**Fix:** fix the syntax error at the reported line and column in the config file.

See [Config Loaders — `ConfigLoaderParseError`](config-loaders.md#configloaderparseError).

---

### `ConfigLoaderPathBackslash`

**When:** The loader path literal contains a backslash `\`.

**Message:** `loader paths use '/' as the path separator; found '\' in {path}`

**Fix:** replace `\` with `/` in the path literal.

See [Config Loaders — `ConfigLoaderPathBackslash`](config-loaders.md#configloaderpathbackslash).

---

### `ConfigLoaderPathEscapesWorkspace`

**When:** The path is absolute, contains a `..` escape, or uses a scheme prefix.

**Message:** `loader path must be a workspace-relative path; found {path}`

**Fix:** use a path relative to the workspace root with no `..` segments.

See [Config Loaders — `ConfigLoaderPathEscapesWorkspace`](config-loaders.md#configloaderpathescapesworkspace).

---

### `ConfigLoaderPathNotLiteral`

**When:** The `path` argument to a loader is not a string literal.

**Message:** `loader path must be a string literal; found {expr}`

**Fix:** replace the argument with a string literal: `'configs/data.yaml'`.

See [Config Loaders — `ConfigLoaderPathNotLiteral`](config-loaders.md#configloaderpathnot-literal).

---

### `ConfigLoaderRequiredFieldMissing`

**When:** A record entry in the loaded file omits a field required by the schema.

**Message:** `field '{name}' required by schema is missing`

**Fix:** add the missing field to the YAML/JSON entry, or remove it from the schema.

See [Config Loaders — `ConfigLoaderRequiredFieldMissing`](config-loaders.md#configloaderrequiredfieldmissing).

---

### `ConfigLoaderRootShapeMismatch`

**When:** The file's top-level shape (sequence, mapping, scalar) does not match the schema's expected root shape.

**Message:** `schema '{type}' expects {expected_shape}; file's top level is {actual_shape}`

**Fix:** align the schema and the file — use `List<S>` for a sequence root, `{…}` or `Map<Text, S>` for a mapping root.

See [Config Loaders — `ConfigLoaderRootShapeMismatch`](config-loaders.md#configloaderrootshapemismatch).

---

### `ConfigLoaderSchemaForbidden`

**When:** The schema argument is not an admissible shape (e.g. a bare scalar like `Integer`).

**Message:** `loader schema must be a record type, 'List<record>', or 'Map<Text, record>'; found {actual}`

**Fix:** use a record schema (`{…}` or a named `smelt.record`), `List<{…}>`, or `Map<Text, {…}>`.

See [Config Loaders — `ConfigLoaderSchemaForbidden`](config-loaders.md#configloaderschemaforbidden).

---

### `ConfigLoaderTomlNotYetSupported`

**When:** `smelt.config.load_toml` is called.

**Message:** `smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1`

**Fix:** convert the config file to YAML or JSON.

See [Config Loaders — `ConfigLoaderTomlNotYetSupported`](config-loaders.md#configloadertomlnotyetsupported).

---

### `ConfigLoaderTypeMismatch`

**When:** A field value in the loaded file is not assignable to the declared field type.

**Message:** `field '{name}' expects {expected}; got {actual}`

**Fix:** correct the value in the YAML/JSON file to match the declared field type.

See [Config Loaders — `ConfigLoaderTypeMismatch`](config-loaders.md#configloadertypemismatch).

---

### `ConfigLoaderUnknownField`

**When:** A record entry in the loaded file contains a field not declared in the schema.

**Message:** `field '{name}' is not declared in the schema; expected one of: {fields}`

**Fix:** remove the extra field from the YAML/JSON entry, or add it to the schema.

See [Config Loaders — `ConfigLoaderUnknownField`](config-loaders.md#configloaderunknownfield).

---

### `ConfigVarNameNotLiteral`

**When:** The argument to `smelt.config.var` is not a string literal.

**Message:** `smelt.config.var name must be a string literal`

**Fix:** use a string literal: `smelt.config.var('my_var')`.

See [Config Variables — `ConfigVarNameNotLiteral`](config-vars.md#configvarnamenotliteral).

---

### `ConfigVarNotFound`

**When:** `smelt.config.var('name')` is called but `name` is not declared in `smelt.yml` `vars:`.

**Message:** `compile-time variable {name} not declared in smelt.yml vars`

**Fix:** add `name:` under `vars:` in `smelt.yml`, or check for typos in the variable name.

See [Config Variables — `ConfigVarNotFound`](config-vars.md#configvarnotfound).

---

### `ConfigVarNullCoercion` (warning)

**When:** A `vars:` entry has a YAML `null` value, coerced to `''` at the call site.

**Message:** `null variable {name} coerced to empty string; declare a default in smelt.yml`

**Fix:** replace the null YAML value with an explicit default string.

See [Config Variables — `ConfigVarNullCoercion`](config-vars.md#configvarnullcoercion-warning).

---

### `GenerateFileBareSelectForbidden`

**When:** A generator file (frontmatter `generates: models`) contains a top-level bare `SELECT`, `WITH`, or `VALUES` statement instead of a meta-language expression.

**Message:** `generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape`

**Fix:** replace the bare statement with a meta-expression of type `List<ModelDef>`, or remove `generates: models` from the frontmatter.

See [Generator Files](generators.md).

---

### `GenerateFileBodyTypeError`

**When:** The generator file body evaluates to a type that is not assignable to `List<ModelDef>`.

**Message:** `generator file body must evaluate to List<ModelDef>; found {actual}`

**Fix:** ensure the body expression produces a `List<ModelDef>`.

See [Generator Files](generators.md).

---

### `GeneratesMixedWithBareModel`

**When:** `generates: models` appears in a file that also has a `name:` frontmatter field or Layer-1 section delimiters.

**Message:** `generates: models is mutually exclusive with name: or Layer-1 delimiters`

**Fix:** a file is either a generator (no `name:`, no Layer-1 delimiters) or a bare model — not both.

See [Generator Files](generators.md).

---

### `GeneratesUnknownValue`

**When:** The `generates:` frontmatter key carries a value other than `models`.

**Message:** `generates: expects value 'models'; found '{actual}'`

**Fix:** use `generates: models`, or remove the `generates:` key.

See [Generator Files](generators.md).

---

### `GeneratorBodyForbidsModelReflection`

**When:** A generator file body calls `smelt.models.with_tag` or `smelt.models.all`.

**Message:** `smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references`

**Fix:** drive generation from `smelt.config.load_yaml` / `load_json`, from `smelt.sources.*`, or from literal `smelt.<path>` references to hand-authored models.

See [Generator Files — Generator-body reflection restriction](generators.md#generator-body-reflection-restriction).

---

### `HofExpectsLambda`

**When:** The second argument to `map` or `filter` is not a lambda.

**Message:** `{hof} expects a lambda; found {actual type}`

**Fix:** replace the second argument with a `fn x => body` lambda.

See [Higher-Order Functions — `HofExpectsLambda`](hofs.md#hofexpectslambda).

---

### `HofExpectsReducer`

**When:** The second argument to `reduce` is not a bare reducer identifier from the closed registry.

**Message:** `reduce expects a reducer; found {actual}`

**Fix:** use one of the seven registered reducer names. See [Reducers](reducers.md) for the full list.

See [Higher-Order Functions — `HofExpectsReducer`](hofs.md#hofexpectsreducer).

---

### `HofNameShadowed`

**When:** A `smelt.define` function is declared with the name `map`, `filter`, or `reduce`.

**Message:** `{name} is a reserved higher-order function name`

**Fix:** rename the `smelt.define` function.

See [Higher-Order Functions — `HofNameShadowed`](hofs.md#hofnameshadowed).

---

### `LambdaArityMismatch`

**When:** The lambda's parameter count does not match the arity required by the surrounding HOF. `map` and `filter` require arity 1; a multi-arg lambda passed to either emits this diagnostic.

**Message:** `{hof} expects a lambda of arity {expected}; found arity {actual}`

**Fix:** match the lambda's parameter count to what the HOF requires. For `map`/`filter`, use a single-parameter lambda.

See [Lambdas — `LambdaArityMismatch`](lambdas.md#lambdaaritymismatch).

---

### `LambdaDuplicateParameter`

**When:** The same parameter name appears more than once in a single lambda's parameter list.

**Message:** `parameter '{name}' already appears in this lambda's parameter list`

**Fix:** give each parameter a distinct name.

See [Lambdas — `LambdaDuplicateParameter`](lambdas.md#lambdaduplicateparameter).

---

### `LambdaInForbiddenPosition`

**When:** A `fn x => body` lambda appears outside a HOF positional argument position.

**Message:** `lambda is only valid as an argument to a higher-order function`

**Fix:** move the lambda inside a `map` or `filter` call.

See [Lambdas — `LambdaInForbiddenPosition`](lambdas.md#lambdainforbiddenposition).

---

### `LambdaResultTypeMismatch`

**When:** The lambda body's type is incompatible with what the surrounding HOF requires (e.g. `filter` requires `Boolean`).

**Message:** `{hof} requires lambda result {expected}; found {actual}`

**Fix:** adjust the body expression to produce the required type.

See [Lambdas — `LambdaResultTypeMismatch`](lambdas.md#lambdaresulttypemismatch).

---

### `LambdaZeroParameters`

**When:** A lambda's parameter list is empty: `fn () => body`.

**Message:** `lambda must declare at least one parameter`

**Fix:** add at least one parameter; if the body does not use it, name it `_` by convention.

See [Lambdas — `LambdaZeroParameters`](lambdas.md#lambdazeroparameters).

---

### `ModelDefDuplicateName`

**When:** Two `ModelDef` values in the same generator file share the same `name` field value.

**Message:** `duplicate ModelDef.name '{name}' in this generator file`

**Fix:** give each emitted model a unique `name` value within the file.

See [Generator Files — Name uniqueness and collision rules](generators.md#name-uniqueness-and-collision-rules).

---

### `ModelDefHandAuthoredCollision`

**When:** A generator-emitted model's smelt path collides with a hand-authored model or with another generator's emission.

**Message:** `ModelDef emits '{smelt_path}' which collides with {other_path}`

**Fix:** rename the `ModelDef` to produce a unique smelt path, or remove the conflicting hand-authored model.

See [Generator Files — Name uniqueness and collision rules](generators.md#name-uniqueness-and-collision-rules).

---

### `ModelDefInvalidMaterialization`

**When:** The `materialization` field of a `ModelDef` literal contains a value that is not a known materialization strategy.

**Message:** `invalid ModelDef.materialization '{value}'; expected one of: view, table, incremental`

**Fix:** use one of the three valid values: `'view'`, `'table'`, or `'incremental'`.

See [Generator Files](generators.md).

---

### `ModelDefInvalidName`

**When:** The `name` field of a `ModelDef` literal contains a value that is not a path-safe identifier (ASCII letters, digits, underscores; must not start with a digit).

**Message:** `ModelDef.name '{value}' is not a valid identifier`

**Fix:** use a name containing only `[a-zA-Z_][a-zA-Z0-9_]*`.

See [Generator Files](generators.md).

---

### `ModelDefOutsideGeneratorFile`

**When:** A `ModelDef { … }` record literal appears outside a generator file body — in a hand-authored model, a `smelt.define` function body, or any other non-generator context.

**Message:** `ModelDef literals are only valid inside a \`generates: models\` file body`

**Fix:** move the `ModelDef` literal into a generator file (frontmatter `generates: models`).

See [Generator Files](generators.md).

---

### `MapApiArgTypeMismatch`

**When:** `m.get(k)` or `m.has(k)` is called with an argument whose type is not assignable to the map's key type `K`.

**Message:** `Map.{method} expects key of type {expected}; found {actual}`

**Fix:** pass a `Text`-typed key, or cast the argument to `Text`.

See [Maps — Diagnostic codes](maps.md#diagnostic-codes).

---

### `MapApiArityMismatch`

**When:** `m.get` or `m.has` is called with other than one positional argument.

**Message:** `Map.{method} expects one positional argument; found {n}`

**Fix:** pass exactly one positional key argument.

See [Maps — Diagnostic codes](maps.md#diagnostic-codes).

---

### `MapApiNamedArgument`

**When:** A Map API method is called with a named argument.

**Message:** `Map.{method} does not support named arguments`

**Fix:** use positional syntax: `m.get('key')` not `m.get(key => 'key')`.

See [Maps — Diagnostic codes](maps.md#diagnostic-codes).

---

### `MapApiUnexpectedArgument`

**When:** `m.entries`, `m.keys`, or `m.values` is called with any argument.

**Message:** `Map.{method} takes no arguments`

**Fix:** call the method with no arguments: `m.entries()`.

See [Maps — Diagnostic codes](maps.md#diagnostic-codes).

---

### `MapApiUnknown`

**When:** A method call on a `Map<K, V>` value uses a name outside the closed API (`entries`, `keys`, `values`, `get`, `has`).

**Message:** `Map has no method '{name}'; expected one of: entries, keys, values, get, has`

**Fix:** use one of the five supported method names.

See [Maps — Diagnostic codes](maps.md#diagnostic-codes).

---

### `MapGetMissingKey`

**When:** `m.get(k)` is called with a string literal `k` that is statically known to be absent from `m`.

**Message:** `Map has no binding for key '{key}'`

**Fix:** guard with `m.has(k)`, use a key that is declared in the YAML file, or handle the absent case explicitly.

See [Maps — `m.get` missing-key behaviour](maps.md#mget-missing-key-behaviour).

---

### `MapKeyTypeNotText`

**When:** A `Map<K, V>` type expression is written where `K` is not `Text`.

**Message:** `Map key type must be Text in v1; found {type}`

**Fix:** use `Map<Text, V>`. Non-`Text` key types are reserved for a future version.

See [Maps — The `Map<K, V>` type](maps.md#the-mapk-v-type).

---

### `MetaListEmptyTypeUnknown`

**When:** A bare `[]` literal appears where the type checker cannot infer the element type from context.

**Message:** `cannot infer element type for empty list literal`

**Fires at:** the `[]` CST span.

**Example:**
```sql
SELECT
    id,
    []   -- MetaListEmptyTypeUnknown
FROM smelt.sources.raw.users
```

**Fix:** provide elements so the type can be inferred; use `...[]` (spread of an empty list) for silent elision; or annotate a `smelt.define` parameter with the expected `List<T>` type so the empty literal has a target sort.

---

### `MetaListHeterogeneous`

**When:** The elements of a list literal do not share a common type under LUB.

**Message:** `list elements have incompatible types: {T0}, {Tk}`

**Fires at:** the offending list literal CST span.

**Example:**
```sql
SELECT id, ...[1, 'hello'] FROM smelt.sources.raw.users
--              ^^^^^^^^^^^  MetaListHeterogeneous: INTEGER vs TEXT
```

**Fix:** ensure all elements share a compatible type. Numeric mixed precision is promoted automatically (`[1, 2.5]` infers `DECIMAL`). For truly incompatible types, cast all elements to a common type or split into separate lists.

---

### `MetaSpreadInForbiddenPosition`

**When:** A `...xs` spread operator appears in a grammar position that does not support spread. Forbidden positions: WHERE clause, FROM clause without an explicit reducer, boolean-composition context (`AND`/`OR`), named-argument position.

**Message:** `spread is not allowed in {position name}`

**Fires at:** the `...` CST span.

**Example:**
```sql
SELECT id
FROM smelt.sources.raw.users
WHERE id = 1 AND ...preds  -- MetaSpreadInForbiddenPosition
```

**Fix:** move the spread to a SELECT list. For WHERE-clause predicate lists, use the `and_all` reducer.

---

### `MetaSpreadOnNonList`

**When:** The `...` operator is applied to an expression that does not have type `List<T>`.

**Message:** `spread expects List<T>; found {actual type}`

**Fires at:** the `...` CST span.

**Example:**
```sql
SELECT id, ...some_integer FROM smelt.sources.raw.users
--         ^^^^^^^^^^^^^^  MetaSpreadOnNonList: INTEGER is not List<T>
```

**Fix:** wrap the value in a list literal (`...[some_integer]`) to splice a single element, or verify that the binding supplying the value actually has type `List<T>`.

---

### `ModelRefFieldUnknown`

**When:** Field access on a `ModelRef` value uses an identifier that is not one of the four declared fields (`path`, `name`, `tags`, `columns`).

**Message:** ``ModelRef has no field `{name}`; expected one of: path, name, tags, columns``

**Fix:** use `m.path` (Text), `m.name` (Text), `m.tags` (List\<Text\>), or `m.columns` (List\<ColumnRef\>). Any other field requires a spec extension.

See [Reflection — `ModelRefFieldUnknown`](reflection.md#modelreffieldunknown).

---

### `PipeInDataPosition`

**When:** A `|>` pipe expression appears in a Data-World grammar position.

**Message:** `|> is meta-only; use SQL composition in this position`

**Fix:** move the pipe chain to a meta-world context, or use SQL operators directly in the data position.

See [Pipe Operator — `PipeInDataPosition`](pipes.md#pipeindataposition).

---

### `PipeRhsNotCall`

**When:** The right-hand side of `|>` is not a function call expression.

**Message:** `pipe right-hand side must be a function call`

**Fix:** write the RHS as a call: `LHS |> f(args)`.

See [Pipe Operator — `PipeRhsNotCall`](pipes.md#piperhsnotcall).

---

### `RecordCyclicDeclaration`

**When:** A `smelt.record` declaration references its own name (directly or transitively through other declarations), forming a cycle.

**Message:** `record '{name}' is cyclic; record declarations must form a DAG`

**Fix:** break the cycle by extracting a shared base record. Mutually recursive records are not supported in v1.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldDuplicate`

**When:** A record literal names the same field twice.

**Message:** `field '{name}' already appears in this record literal`

**Fix:** remove the duplicate field occurrence from the literal.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldMissing`

**When:** A record literal omits a field required by the target type.

**Message:** `record literal for '{type}' is missing required field '{name}'`

**Fix:** add the missing field, or use a narrower inline record type that does not require it.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldNotProjectable`

**When:** Mid-chain field projection steps through a non-record-typed value.

**Message:** `value of type {type} has no fields; projection '{field}' is not valid`

**Fix:** stop the projection at the correct depth. The intermediate value is not a record.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldTypeForbidden`

**When:** A `smelt.record` field type references a reflection witness (`ColumnRef`, `ModelRef`, `SourceRef`, or `Lambda<…>`).

**Message:** `record field types may not reference {type}; reflection witnesses are not user-writable`

**Fix:** use a concrete `DataType` or a user-authored record type as the field type.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldTypeMismatch`

**When:** A field value in a record literal is not assignable to the declared field type.

**Message:** `record field '{name}' expects {expected}; found {actual}`

**Fix:** cast or replace the value so it matches the declared field type.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordFieldUnknown`

**When:** A field projection or record literal uses a field name not declared on the target record type.

**Message:** `record '{type}' has no field '{name}'; expected one of: {fields}`

**Fix:** use one of the declared field names, or add the field to the `smelt.record` declaration.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordInDataWorld`

**When:** A record-typed binding is referenced bare in a Data-World SQL position (e.g. a `WHERE` clause or a SELECT item) without projecting a field.

**Message:** `record-typed value '{name}' has no Data-World representation; project a field (e.g. '{name}.field') or consume it inside a meta-language splice`

**Fix:** project a scalar field of the record (`r.field`) instead of using the bare binding. Records live in the meta-world; their projected fields cross into the Data-World.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `RecordLiteralUnknownTarget`

**When:** A record literal `{…}` appears in a position where no target type can be inferred.

**Message:** `cannot infer record type from context; annotate the target type`

**Fix:** provide context — pass the literal as a loader schema argument, annotate a `smelt.define` parameter, or wrap in a typed position.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `ReducerEmptyNoIdentity`

**When:** `reduce` is called with an empty list using `union_all` or `intersect_all`, which have no identity element.

**Message:** `reducer {r} has no identity for an empty list`

**Fix:** ensure the source list is non-empty, or use a reducer that has an empty-list identity.

See [Reducers — `ReducerEmptyNoIdentity`](reducers.md#reduceremptynoidentity).

---

### `ReducerInputTypeMismatch`

**When:** `reduce` is called with a list whose element type is incompatible with the reducer's declared input.

**Message:** `reducer {r} expects List<{T_in}>; found List<{T_actual}>`

**Fix:** use `map` to convert the list elements to the correct type first, or choose a different reducer.

See [Reducers — `ReducerInputTypeMismatch`](reducers.md#reducerinputtypemismatch).

---

### `ReducerArgNotCompileTime`

**When:** A parameterised reducer argument is not a compile-time-resolvable meta value.

**Message:** `reducer {r}'s argument '{param}' must be a compile-time value; found {actual}`

**Fix:** replace the runtime argument with a compile-time value — a string literal or a `smelt.config.var(...)` result.

See [Reducers — `ReducerArgNotCompileTime`](reducers.md#reducerargnotcompiletime).

---

### `ReducerArgTypeMismatch`

**When:** A parameterised reducer argument's type is not assignable to the declared parameter type.

**Message:** `reducer {r}'s argument '{param}' expects {expected}; found {actual}`

**Fix:** pass a value of the declared parameter type. For `concat_with`, the separator must be `Text`.

See [Reducers — `ReducerArgTypeMismatch`](reducers.md#reducerargtypemismatch).

---

### `ReducerArityMismatch`

**When:** A parameterised reducer call has the wrong number of positional arguments.

**Message:** `reducer {r} expects {expected} argument(s); found {actual}`

**Fix:** provide the correct number of arguments. `concat_with` requires exactly one.

See [Reducers — `ReducerArityMismatch`](reducers.md#reduceraritymismatch).

---

### `ReducerNamedArgument`

**When:** A parameterised reducer is called with a named argument (`sep => ', '`) instead of a positional one.

**Message:** `reducer {r} takes positional arguments only`

**Fix:** use positional syntax: `concat_with(', ')`.

See [Reducers — `ReducerNamedArgument`](reducers.md#reducernamedargument).

---

### `ReducerNameShadowed`

**When:** A `smelt.define` function is declared with a name that matches one of the seven reserved reducer names.

**Message:** `{name} is a reserved reducer name`

**Fix:** rename the `smelt.define` function.

See [Reducers — `ReducerNameShadowed`](reducers.md#reducernameshadowed).

---

### `SmeltRecordRedefinition`

**When:** A `smelt.record` declaration uses a name that is already declared elsewhere in the workspace.

**Message:** `record '{name}' is already declared in {path}; record names must be unique workspace-wide`

**Fix:** choose a distinct name for the second declaration, or consolidate both into one.

See [Records — Diagnostic codes](records.md#diagnostic-codes).

---

### `TernaryBranchTypeMismatch`

**When:** The `THEN_EXPR` and `ELSE_EXPR` branches synthesise to types that do not unify under LUB.

**Message:** `ternary branches have incompatible types: {then_type} vs {else_type}`

**Fix:** ensure both branches produce values of compatible types. Numeric types are promoted automatically; incompatible sorts (e.g. `INTEGER` vs `TEXT`) require restructuring the logic.

See [Ternary — `TernaryBranchTypeMismatch`](ternary.md#ternarybranchtypemismatch).

---

### `TernaryConditionNotBoolean`

**When:** The `COND` expression in `if COND then … else …` synthesises to a type that is not assignable to `Boolean`.

**Message:** `ternary condition expects Boolean; found {actual}`

**Fix:** use a Boolean expression as the condition — a comparison, `m.has(k)`, or a `Boolean`-typed `smelt.config.var` result. The meta-language has no Boolean coercion.

See [Ternary — `TernaryConditionNotBoolean`](ternary.md#ternaryconditionnotboolean).

---

### `TernaryDanglingElse`

**When:** An `else` keyword appears outside any in-progress ternary's `THEN_EXPR` slot — with no preceding `if … then`, or after the ternary has already consumed its `else` clause.

**Message:** `unexpected 'else' keyword outside of '... then ... else' form`

**Fix:** remove the extra `else` clause, or restructure the ternary chain.

See [Ternary — `TernaryDanglingElse`](ternary.md#ternarydanglingelse).

---

### `TernaryDanglingThen`

**When:** A `then` keyword appears outside any in-progress ternary's `COND` slot — for example with no preceding `if`, or after the `then` slot has already been consumed.

**Message:** `unexpected 'then' keyword outside of 'if ... then ...' form`

**Fix:** add the missing `if COND` prefix, or remove the stray `then`.

See [Ternary — `TernaryDanglingThen`](ternary.md#ternarydanglingthen).

---

### `TernaryInDataPosition`

**When:** An `if … then … else …` ternary expression appears in a Data-World grammar position that does not admit meta evaluation.

**Message:** `if-then-else is meta-only; use SQL CASE WHEN in this position`

**Fix:** replace the ternary with a SQL `CASE WHEN … THEN … ELSE … END` expression in the data position.

See [Ternary — `TernaryInDataPosition`](ternary.md#ternaryindataposition).

---

### `TernaryKeywordShadowed`

**When:** A `smelt.define` function, `smelt.record` declaration, or lambda parameter is declared with the name `if`, `then`, or `else`.

**Message:** `{name} is a reserved meta-language keyword`

**Fix:** choose a different name. The three keywords are reserved at the meta-namespace level.

See [Ternary — `TernaryKeywordShadowed`](ternary.md#ternarykeywordshadowed).

---

### `SourceRefFieldUnknown`

**When:** Field access on a `SourceRef` value uses an identifier that is not one of the four declared fields (`path`, `name`, `tags`, `columns`).

**Message:** ``SourceRef has no field `{name}`; expected one of: path, name, tags, columns``

**Fix:** use `s.path` (Text), `s.name` (Text), `s.tags` (List\<Text\>), or `s.columns` (List\<ColumnRef\>). Any other field requires a spec extension.

See [Reflection — `SourceRefFieldUnknown`](reflection.md#sourcereffieldunknown).

---

### `WithTagNamedArgument`

**When:** `smelt.models.with_tag` or `smelt.sources.with_tag` is called with a named argument instead of a positional one.

**Message:** `with_tag takes one positional argument; named arguments are not supported`

**Fix:** use positional syntax: `with_tag('my-tag')` not `with_tag(tag => 'my-tag')`.

See [Reflection — `WithTagNamedArgument`](reflection.md#withtagnamedargument).

---

### `WithTagRequiresText`

**When:** The argument to `smelt.models.with_tag` or `smelt.sources.with_tag` is not a compile-time string literal (e.g. it is an integer or a runtime expression like `UPPER('cohort')`).

**Message:** `with_tag expects a compile-time Text; found {actual}`

**Fix:** pass a string literal: `with_tag('my-tag')`. Dynamic tag filtering is not supported.

See [Reflection — `WithTagRequiresText`](reflection.md#withtagrequirestext).

---

### `WideReflectionUnexpectedArgument`

**When:** `smelt.models.all` or `smelt.sources.all` is called with one or more arguments.

**Message:** `{accessor} takes no arguments`

**Fix:** call `all()` with no arguments.

See [Reflection — `WideReflectionUnexpectedArgument`](reflection.md#widereflectionunexpectedargument).

---

### `WideReflectionUnknownAccessor`

**When:** An unknown accessor is used under `smelt.models.*` or `smelt.sources.*` (e.g. `smelt.models.bogus()`).

**Message:** ``smelt.{models,sources} has no accessor `{name}`; expected one of: with_tag, all``

**Fix:** use only `with_tag('tag')` or `all()`.

See [Reflection — `WideReflectionUnknownAccessor`](reflection.md#widereflectionunknownaccessor).
