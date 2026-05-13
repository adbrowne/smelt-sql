# Maps

The meta-language provides **`Map<K, V>`** — a compile-time key-value collection produced by the config-loader family when the declared schema is a mapping type. Maps are meta-only: no `Map<K, V>` value ever reaches the database engine.

## The `Map<K, V>` type

`Map<K, V>` is a meta-only type. A `Map<K, V>` value is:

- **Finite** — the key set is fixed at construction (load time).
- **Keyed** — each key maps to exactly one value.
- **Immutable** — no key can be added, removed, or replaced after construction.
- **Compile-time-only** — no `Map<K, V>` value ever reaches the database engine.

In v1 the key type `K` must be `Text`. A `Map<K, V>` type expression where `K` is anything other than `Text` emits `MapKeyTypeNotText` at the type expression. Future versions may relax this constraint when an equality-and-hashing surface is defined for additional key types.

`Map<K, V>` is **invariant** in both `K` and `V`. `Map<Text, Integer>` is not a `Map<Text, Number>` even though `Integer <: Number` in the numeric chain. The invariance protects key-lookup semantics.

**Iteration order** for `m.entries()`, `m.keys()`, and `m.values()` is **byte-lexicographic ascending by key**. This order is deterministic and independent of YAML/JSON key order in the source file.

## Loader-only origin

There is **no `Map<…>` literal syntax** in v1. Maps are produced exclusively by the config-loader family when the schema argument is `Map<Text, S>`:

```sql
-- Load a YAML mapping → Map<Text, {plan: Text, threshold: Integer}>
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>)
```

A user constructing a key-value structure inline writes a `List<{key: Text, value: V}>` — the same shape that `m.entries()` produces — and consumes it with the existing list HOFs.

See [Config Loaders](config-loaders.md) for the full loader API.

## Map API

Operations on a `Map<Text, V>` value `m` use method-call syntax. The five method names `entries`, `keys`, `values`, `get`, `has` form the **closed Map API**; any other method name emits `MapApiUnknown`.

| Method | Signature | Result |
|--------|-----------|--------|
| `m.entries()` | `Map<K, V> -> List<{key: K, value: V}>` | Key-value pairs, sorted ascending by key |
| `m.keys()` | `Map<K, V> -> List<K>` | Keys only, sorted ascending |
| `m.values()` | `Map<K, V> -> List<V>` | Values only, ordered by their keys' ascending sort |
| `m.get(k)` | `(Map<K, V>, K) -> V` | The value bound to key `k` |
| `m.has(k)` | `(Map<K, V>, K) -> Boolean` | `TRUE` iff `m` contains `k` |

**Argument rules:**
- `m.get(k)` and `m.has(k)` require exactly one positional argument. Arity mismatches emit `MapApiArityMismatch`.
- `m.entries()`, `m.keys()`, and `m.values()` require no arguments. Any argument emits `MapApiUnexpectedArgument`.
- No Map API method supports named arguments. Named arguments emit `MapApiNamedArgument`.
- The argument to `m.get` and `m.has` must be assignable to `K`. Type mismatches emit `MapApiArgTypeMismatch`.

## `m.get` missing-key behaviour

`m.get(k)` behaviour depends on whether `k` is statically resolvable:

| Situation | Outcome |
|-----------|---------|
| `k` is a string literal known to be absent from `m` at compile time | `MapGetMissingKey` diagnostic; call evaluates to `Unknown` |
| `k` is a string literal known to be present in `m` | Evaluates to the bound value typed `V`; no diagnostic |
| `k` is a non-literal expression (not statically resolvable) | Type is `V`; evaluation is deferred to expansion time |

This means that `m.get('missing_key')` on a statically-known map is a **compile-time error**, not a silent `Unknown`. Use `m.has(k)` to guard the lookup if the key may be absent.

## Worked example

The `examples/meta_config/` workspace loads a tenant configuration map:

```sql
-- examples/meta_config/models/tenants.sql
smelt.record Tenant = { plan: Text, threshold: Integer }

SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>)
```

With `configs/tenants.yaml`:

```yaml
tenant_a:
  plan: pro
  threshold: 100
tenant_b:
  plan: free
  threshold: 10
```

The loader validates each mapping entry against the inline schema `{plan: Text, threshold: Integer}` and returns `Map<Text, {plan: Text, threshold: Integer}>`. To produce a SELECT over all tenants:

```sql
smelt.record Tenant = { plan: Text, threshold: Integer }

SELECT
    smelt.config.load_yaml('configs/tenants.yaml', Map<Text, Tenant>)
    |> m => m.entries()
    |> map(fn e => e.key || ': ' || e.value.plan)
```

`m.entries()` returns `List<{key: Text, value: Tenant}>` sorted by key byte-lexicographically — so `tenant_a` appears before `tenant_b`.

**Checking key presence before lookup:**

```sql
-- m.has(k) guards before m.get(k) when the key may be absent
SELECT
    CASE WHEN m.has('tenant_c')
         THEN m.get('tenant_c').plan
         ELSE 'unknown'
    END
```

## LSP support

- **Hover** on a `Map<K, V>`-typed binding shows the type, the resolved entry count when statically known, and the first five keys.
- **Hover** on a method invocation (`m.entries`, `m.keys`, etc.) shows the method's signature and, when the underlying `Map` is statically resolvable, the result length.
- **Hover** on `m.get(k)` shows the result type `V` and, when `k` is statically known and present, the resolved value's hover.
- **Goto-definition** on a Map API method name resolves to the reference page (graceful no-op when the client lacks support).
- **Completion** at `m.<cursor>` offers the five Map API method names with signatures.
- **Completion** at `m.get(<cursor>)` and `m.has(<cursor>)` offers the statically-known key list when resolvable.

## Diagnostic codes

---

!!! warning "MapKeyTypeNotText"
    **When it fires:** A `Map<K, V>` type expression is written where `K` is not `Text`.

    **Message:** `Map key type must be Text in v1; found {type}`

    **Example:**
    ```sql
    -- Map<Integer, Text> is not supported in v1
    SELECT smelt.config.load_yaml('data.yaml', Map<Integer, {name: Text}>)
    -- ← MapKeyTypeNotText
    ```

    **What to fix:** Use `Map<Text, V>`. Non-`Text` key types are reserved for a future version.

---

!!! warning "MapApiUnknown"
    **When it fires:** A method call on a `Map<K, V>` value uses a name outside the closed API (`entries`, `keys`, `values`, `get`, `has`).

    **Message:** `Map has no method '{name}'; expected one of: entries, keys, values, get, has`

    **Example:**
    ```sql
    SELECT m.each()  -- ← MapApiUnknown: 'each' is not in the closed API
    ```

    **What to fix:** Use one of the five supported method names.

---

!!! warning "MapApiArityMismatch"
    **When it fires:** `m.get` or `m.has` is called with other than one positional argument.

    **Message:** `Map.{method} expects one positional argument; found {n}`

    **Example:**
    ```sql
    SELECT m.get('a', 'b')  -- ← MapApiArityMismatch: two arguments
    SELECT m.has()          -- ← MapApiArityMismatch: no arguments
    ```

    **What to fix:** Pass exactly one positional key argument.

---

!!! warning "MapApiNamedArgument"
    **When it fires:** A Map API method is called with a named argument.

    **Message:** `Map.{method} does not support named arguments`

    **Example:**
    ```sql
    SELECT m.get(key => 'tenant_a')  -- ← MapApiNamedArgument
    ```

    **What to fix:** Use positional syntax: `m.get('tenant_a')`.

---

!!! warning "MapApiUnexpectedArgument"
    **When it fires:** `m.entries`, `m.keys`, or `m.values` is called with any argument.

    **Message:** `Map.{method} takes no arguments`

    **Example:**
    ```sql
    SELECT m.entries('extra')  -- ← MapApiUnexpectedArgument
    ```

    **What to fix:** Call the method with no arguments: `m.entries()`.

---

!!! warning "MapGetMissingKey"
    **When it fires:** `m.get(k)` is called with a string literal `k` that is statically known to be absent from `m`.

    **Message:** `Map has no binding for key '{key}'`

    **Example:**
    ```sql
    -- configs/tenants.yaml has keys 'tenant_a' and 'tenant_b'
    SELECT m.get('tenant_c')  -- ← MapGetMissingKey: 'tenant_c' is not present
    ```

    **What to fix:** Guard the lookup with `m.has(k)`, use a key that is declared in the YAML file, or handle the absent case explicitly.

---

!!! warning "MapApiArgTypeMismatch"
    **When it fires:** `m.get(k)` or `m.has(k)` is called with an argument whose type is not assignable to `K`.

    **Message:** `Map.{method} expects key of type {expected}; found {actual}`

    **Example:**
    ```sql
    -- Map<Text, Tenant>: key must be Text
    SELECT m.get(42)  -- ← MapApiArgTypeMismatch: Integer is not Text
    ```

    **What to fix:** Pass a `Text`-typed key (a string literal or a `Text`-valued meta expression).
