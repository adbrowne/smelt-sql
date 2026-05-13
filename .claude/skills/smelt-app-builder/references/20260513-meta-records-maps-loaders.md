# Records, Maps, and Config Loaders

When building smelt projects that use `smelt.record`, `Map<K, V>`, or
`smelt.config.load_yaml` / `smelt.config.load_json`, the non-obvious behaviours
below are the most common source of confusion.

## Loader path arguments must be string literals

`smelt.config.load_yaml(path, schema)` requires `path` to be a **string
literal** in the source — `'configs/tenants.yaml'`, not a variable or expression.
Using a variable emits `ConfigLoaderPathNotLiteral`. This is intentional: the
Salsa input registration happens at parse time, before any meta-evaluation.

```sql
-- OK
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text}>)

-- Error: ConfigLoaderPathNotLiteral
SELECT smelt.config.load_yaml(my_path_var, Map<Text, {plan: Text}>)
```

## Named vs inline schemas: goto-def and hover differ

A **named `smelt.record` declaration** (`smelt.record Tenant = { … }`) gives
the schema a goto-definition target across all call sites that reference the
name. The LSP `hover` on a named-schema call site shows the declaration link.

An **inline schema** (`{plan: Text, threshold: Integer}`) is anonymous — there
is no goto-definition target for the inline `{…}` type. If the same schema
appears in more than one file, prefer a named declaration so the LSP can
cross-reference usages.

```sql
-- Named: hover shows 'Tenant declared in models/tenants.sql'
smelt.record Tenant = { plan: Text, threshold: Integer }
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, Tenant>)

-- Inline: hover shows the structural type only
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>)
```

## Map iteration is byte-lex-sorted by key, not insertion order

`m.entries()`, `m.keys()`, and `m.values()` produce results in **byte-lexicographic
ascending key order**, regardless of the key order in the YAML or JSON source
file. Do not rely on the file's key order being preserved.

```sql
-- configs/tenants.yaml has keys in order: tenant_b, tenant_a
-- m.keys() returns: ['tenant_a', 'tenant_b']  (sorted)
```

## `m.get(k)` on a statically-missing key is a compile-time error

If `k` is a string literal and the map is statically resolvable, a missing key
is **`MapGetMissingKey`** — a diagnostic, not a silent `Unknown`. Guard with
`m.has(k)` when the key may be absent:

```sql
-- Error if 'tenant_c' is not in the map:
SELECT m.get('tenant_c')  -- MapGetMissingKey

-- Correct guard:
SELECT CASE WHEN m.has('tenant_c') THEN m.get('tenant_c').plan ELSE 'none' END
```

For non-literal keys (e.g., a variable bound by a HOF lambda), resolution is
deferred to expansion time and no compile-time `MapGetMissingKey` fires.

## Per-target overlays: file naming and merge semantics

Per-target overlay files use the `<basename>.<target>.<ext>` sibling convention.
Given `configs/cohorts.yaml` and `target: prod`, smelt looks for
`configs/cohorts.prod.yaml`. Merge semantics depend on the schema root shape:

| Root shape | Merge rule |
|---|---|
| Record (`{…}`) | Field-by-field deep merge: overlay fields replace base fields |
| `List<S>` | Full replacement: overlay replaces the entire base list |
| `Map<Text, S>` | Per-key replacement: overlay keys replace base values for those keys; absent keys are taken from the base |

Notably, `List<S>` overlays are **not concatenated** — the overlay list replaces
the base list in its entirety. If you want a merged list, author it explicitly.

## Record field types may not be reflection witnesses

`ColumnRef`, `ModelRef`, `SourceRef`, and `Lambda<…>` cannot be used as field
types in a `smelt.record` declaration. These are meta-only reflection witnesses
that cannot be stored in user-declared record types:

```sql
-- Error: RecordFieldTypeForbidden
smelt.record Bad = { columns: ColumnRef }

-- OK: use a concrete type
smelt.record Config = { column_name: Text, threshold: Integer }
```

## Width subtyping: wider record is the subtype

A record with more fields is a subtype of a record with fewer fields. This means
you can pass a `Tenant`-typed value (declared with `plan` and `threshold`) to a
HOF lambda that only accesses `plan` — no explicit projection needed:

```sql
smelt.record Tenant = { plan: Text, threshold: Integer }

-- fn e => e.plan works on {plan: Text, threshold: Integer} — width subtyping
SELECT m.entries() |> map(fn e => e.value.plan)
```

The inverse is not true: a `{plan: Text}` value cannot satisfy a `Tenant`-typed
position because `threshold` is missing.

## `null` YAML values coerce to empty `Text` with a warning

If a YAML field declared `Text` has value `~` (YAML null), smelt coerces it to
`''` and emits `ConfigLoaderNullCoercion` (warning severity — the model still
compiles). Don't rely on this coercion; declare an explicit default:

```yaml
# Avoid:
name: ~

# Prefer:
name: ''
# or:
name: default_name
```

## `smelt.config.load_toml` is reserved, not available

`smelt.config.load_toml` is a reserved name that emits
`ConfigLoaderTomlNotYetSupported`. Convert your TOML config to YAML or JSON.

See `docs-site/docs/meta-language/records.md`, `docs-site/docs/meta-language/maps.md`,
and `docs-site/docs/meta-language/config-loaders.md` for full surface and worked
examples.
