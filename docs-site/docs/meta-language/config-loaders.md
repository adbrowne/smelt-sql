# Config Loaders

The meta-language provides a **file-loader family** that reads compile-time configuration from disk and returns typed meta-world values. Loaders are the primary source of `Map<Text, V>` and complex `List<Record>` values.

```sql
smelt.config.load_yaml(path, schema)
smelt.config.load_json(path, schema)
```

`smelt.config.load_toml(path, schema)` is **reserved** and not yet available. Calling it emits `ConfigLoaderTomlNotYetSupported`.

## Path argument

The `path` argument must be a **string literal** — a bare `'configs/data.yaml'` in the source. A path that is a variable, a column reference, or any expression emits `ConfigLoaderPathNotLiteral` at the argument.

**Workspace-relative.** Paths are resolved relative to the workspace root. The following all emit `ConfigLoaderPathEscapesWorkspace`:

- Absolute paths: `'/etc/config.yaml'`
- `..`-escape paths: `'../shared/config.yaml'`
- Scheme prefixes: `'s3://bucket/config.yaml'`, `'http://…'`

**Forward slashes only.** Backslashes emit `ConfigLoaderPathBackslash`. Use `'configs/data.yaml'`, not `'configs\data.yaml'`.

**File must exist.** If the resolved file does not exist at type-check time, the loader emits `ConfigLoaderFileNotFound` at the path literal.

**Salsa-tracked.** Every loaded file (base and overlay) is registered as a Salsa input. An edit to the file on disk invalidates all downstream type checks that consume the loaded value.

## Schema argument

The `schema` argument declares the shape the loader should validate the file against and determines the loader's return type.

Admissible schema shapes:

| Schema syntax | Expected file root | Return type |
|---|---|---|
| `{field: Type, …}` (inline record) | YAML/JSON mapping | `{field: Type, …}` |
| `TypeName` (named `smelt.record`) | YAML/JSON mapping | `TypeName` |
| `List<{…}>` or `List<TypeName>` | YAML/JSON sequence | `List<…>` |
| `Map<Text, {…}>` or `Map<Text, TypeName>` | YAML/JSON mapping | `Map<Text, …>` |

Any other schema shape (bare scalar types like `Integer`, `Text`, arbitrary expressions) emits `ConfigLoaderSchemaForbidden` at the schema argument.

**Inline schemas** are anonymous — there is no goto-definition target for the inline `{…}` type. **Named schemas** (`smelt.record TypeName = { … }` declared elsewhere in the workspace) add a goto-definition target and improve hover display across all call sites that reference the name.

For details on inline record types, named declarations, and width subtyping, see [Records](records.md). For `Map<K, V>` type formation and the Map API, see [Maps](maps.md).

## Per-target overlay

When a build target is active (via `smelt build --target prod` or the `target:` field in `smelt.yml`), the loader automatically checks for a sibling file named `<basename>.<target>.<ext>`. If the sibling exists, its contents are **merged** into the base file before validation:

```
configs/cohorts.yaml          # base — always read
configs/cohorts.prod.yaml     # overlay — read only when target=prod
configs/cohorts.dev.yaml      # overlay — read only when target=dev
```

Merge semantics depend on the schema's root shape:

| Root shape | Merge rule |
|---|---|
| Record | Deep merge by field: each field in the overlay replaces the corresponding field in the base; absent fields are taken from the base |
| `List<S>` | The overlay **replaces** the entire base list; no concatenation |
| `Map<Text, S>` | Per-key replacement: each key in the overlay replaces the base's value for that key; keys absent from the overlay are taken from the base |

A target overlay file that fails schema validation emits the same diagnostic family as a base-file mismatch, anchored at the overlay file's offending row.

## Validation diagnostics

The loader validates path, file existence, file format, and schema conformance. Diagnostics are anchored at the offending row in the YAML/JSON file when possible, with a secondary frame at the loader call site.

## Worked examples

**Example — loading a list of records:**

```sql
-- examples/meta_config/models/cohorts.sql
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

SELECT smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>)
```

With `configs/cohorts.yaml`:

```yaml
- name: us_west
  region: us-west-2
  threshold: 100
- name: us_east
  region: us-east-1
  threshold: 100
- name: eu
  region: eu-west-1
  threshold: 50
```

And the per-target overlay `configs/cohorts.prod.yaml` (active when `target: prod`):

```yaml
- name: us_west
  region: us-west-2
  threshold: 200
- name: us_east
  region: us-east-1
  threshold: 200
- name: eu
  region: eu-west-1
  threshold: 100
```

When building with `target: prod`, the overlay replaces the base list entirely — the threshold values are higher. For `target: dev` (or any target without an overlay), the base file is used unchanged.

**Example — loading a map of records:**

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

The loader returns `Map<Text, {plan: Text, threshold: Integer}>`. Iterate over entries with `m.entries()`:

```sql
smelt.record Tenant = { plan: Text, threshold: Integer }

SELECT
    smelt.config.load_yaml('configs/tenants.yaml', Map<Text, Tenant>)
    |> m => m.entries()
    |> map(fn e => e.key || ': ' || e.value.plan)
```

**Example — JSON loader (same API as YAML):**

```sql
SELECT smelt.config.load_json('configs/settings.json', {debug: Boolean, timeout: Integer})
```

## LSP support

- **Hover** on a loader call site shows the resolved file path, the entry count (for `List`/`Map` roots) or the field set (for record roots), and the file's last-modified timestamp.
- **Hover** on the `schema` argument shows the resolved schema — inline structural display for inline schemas, a declaration link for named schemas.
- **Goto-definition** on the path literal resolves to the loaded file (cursor on row 1).
- **Goto-definition** on a named schema argument resolves to the `smelt.record` declaration.
- **Goto-definition** on a record-typed field of a loaded value, projected at the consumer site, resolves to the YAML/JSON row that produced the value when statically traceable.
- **Completion** at the first positional argument offers workspace-relative paths for `.yaml` / `.yml` (for `load_yaml`) or `.json` (for `load_json`) files.
- **Completion** at the second positional argument offers in-scope `smelt.record` names and a stub `{<cursor>}` for inline schemas.

## Diagnostic codes

---

!!! warning "ConfigLoaderPathNotLiteral"
    **When it fires:** The `path` argument to a loader is not a string literal.

    **Message:** `loader path must be a string literal; found {expr}`

    **Example:**
    ```sql
    -- ← ConfigLoaderPathNotLiteral: path must be a literal, not a variable
    SELECT smelt.config.load_yaml(some_variable, {name: Text})
    ```

    **What to fix:** Replace the path argument with a string literal: `smelt.config.load_yaml('configs/data.yaml', …)`.

---

!!! warning "ConfigLoaderPathEscapesWorkspace"
    **When it fires:** The path is absolute, contains a `..` segment that escapes the workspace root, or begins with a scheme prefix (`http://`, `s3://`, etc.).

    **Message:** `loader path must be a workspace-relative path; found {path}`

    **Example:**
    ```sql
    SELECT smelt.config.load_yaml('../shared/config.yaml', {name: Text})
    -- ← ConfigLoaderPathEscapesWorkspace
    ```

    **What to fix:** Use a path relative to the workspace root: `'configs/data.yaml'`. Move shared configs into the workspace, or symlink them.

---

!!! warning "ConfigLoaderPathBackslash"
    **When it fires:** The path literal contains a backslash `\`.

    **Message:** `loader paths use '/' as the path separator; found '\' in {path}`

    **Example:**
    ```sql
    SELECT smelt.config.load_yaml('configs\tenants.yaml', {plan: Text})
    -- ← ConfigLoaderPathBackslash
    ```

    **What to fix:** Replace `\` with `/`: `'configs/tenants.yaml'`.

---

!!! warning "ConfigLoaderFileNotFound"
    **When it fires:** The resolved file does not exist at its workspace-relative path.

    **Message:** `loader file '{path}' not found in workspace`

    **Example:**
    ```sql
    SELECT smelt.config.load_yaml('configs/nonexistent.yaml', {name: Text})
    -- ← ConfigLoaderFileNotFound
    ```

    **What to fix:** Create the missing file, or correct the path to point to an existing file.

---

!!! warning "ConfigLoaderSchemaForbidden"
    **When it fires:** The schema argument is not an admissible shape (bare scalar, `List<Text>`, `Map<Integer, …>`, etc.).

    **Message:** `loader schema must be a record type, 'List<record>', or 'Map<Text, record>'; found {actual}`

    **Example:**
    ```sql
    SELECT smelt.config.load_yaml('configs/data.yaml', Integer)
    -- ← ConfigLoaderSchemaForbidden: scalar schema not allowed
    ```

    **What to fix:** Use a record schema (`{…}` or a named `smelt.record`), `List<{…}>`, or `Map<Text, {…}>`.

---

!!! warning "ConfigLoaderTomlNotYetSupported"
    **When it fires:** `smelt.config.load_toml` is called.

    **Message:** `smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1`

    **Example:**
    ```sql
    SELECT smelt.config.load_toml('configs/settings.toml', {name: Text})
    -- ← ConfigLoaderTomlNotYetSupported
    ```

    **What to fix:** Convert the TOML file to YAML or JSON and use `load_yaml` / `load_json`.

---

!!! warning "ConfigLoaderParseError"
    **When it fires:** The loaded file is not valid YAML or JSON.

    **Message:** `failed to parse {format} file '{path}': {parser_error}`

    Anchored at the YAML/JSON line where parsing failed; secondary frame at the loader call site.

    **What to fix:** Fix the syntax error in the config file. The parser error message includes the line and column where parsing stopped.

---

!!! warning "ConfigLoaderRequiredFieldMissing"
    **When it fires:** A record entry in the loaded file omits a field declared as required by the schema.

    **Message:** `field '{name}' required by schema is missing`

    Anchored at the YAML/JSON row that is missing the field; secondary frame at the loader call site.

    **Example (`configs/incomplete.yaml`):**
    ```yaml
    - name: us_west
      region: us-west-2
      # threshold is absent — schema requires {name, region, threshold}
    ```

    **What to fix:** Add the missing field to the YAML/JSON entry, or remove the field from the schema if it is truly optional.

---

!!! warning "ConfigLoaderUnknownField"
    **When it fires:** A record entry in the loaded file contains a field not declared in the schema.

    **Message:** `field '{name}' is not declared in the schema; expected one of: {fields}`

    Anchored at the unexpected YAML/JSON key; secondary frame at the loader call site.

    **Example (`configs/extra_field.yaml`):**
    ```yaml
    - name: us_west
      region: us-west-2
      threshold: 100
      extra_field: unexpected   # ← ConfigLoaderUnknownField
    ```

    **What to fix:** Remove the extra field from the YAML/JSON entry, or add it to the schema if it should be present.

---

!!! warning "ConfigLoaderTypeMismatch"
    **When it fires:** A field value in the loaded file is not assignable to the declared field type.

    **Message:** `field '{name}' expects {expected}; got {actual}`

    Anchored at the YAML/JSON value; secondary frame at the loader call site.

    **Example (`configs/wrong_type.yaml`):**
    ```yaml
    - name: us_west
      region: us-west-2
      threshold: not_a_number   # ← ConfigLoaderTypeMismatch: Integer expected
    ```

    **What to fix:** Correct the value in the YAML/JSON file to match the declared field type.

---

!!! warning "ConfigLoaderRootShapeMismatch"
    **When it fires:** The file's top-level shape (sequence, mapping, scalar) does not match what the schema expects.

    **Message:** `schema '{type}' expects {expected_shape}; file's top level is {actual_shape}`

    Anchored at the file's first line; secondary frame at the loader call site.

    **Example:** Using a `List<{…}>` schema against a YAML file whose top level is a mapping (not a sequence).

    **What to fix:** Either change the schema to match the file's root shape, or restructure the YAML/JSON file to match the schema.

---

!!! warning "ConfigLoaderDuplicateMapKey"
    **When it fires:** A `Map<Text, S>`-shaped YAML/JSON file contains the same key twice.

    **Message:** `duplicate map key '{key}' at {row}; earlier appearance at {first_row}`

    **What to fix:** Remove the duplicate key from the YAML/JSON file, keeping the intended value.

---

!!! note "ConfigLoaderNullCoercion (warning)"
    **When it fires:** A YAML `null` scalar (`~` or `null`) appears at a schema field declared `Text`. The null coerces to empty string `''`.

    **Message:** `null value at {row} coerced to empty string; declare a default in the source file` (warning severity — the model still compiles)

    **Example (`configs/null_text.yaml`):**
    ```yaml
    - name: ~    # YAML null → coerces to ''
      region: us-west-2
      threshold: 100
    ```

    **What to fix:** Replace the null with an explicit empty string `''` or a meaningful default. Don't rely on the coercion — future spec versions may tighten this to an error.
