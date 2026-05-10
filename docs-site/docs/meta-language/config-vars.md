# Config Variables

Phase B of the meta-language introduces **`smelt.config.var`** — a compile-time variable lookup that reads values from the `vars:` block of your workspace's `smelt.yml`. It is the first Phase B reflection surface: the workspace is known at compile time, and `smelt.config.var` makes that knowledge available as a `Text` value inside your models.

## Syntax

```sql
smelt.config.var('variable_name')
```

- The argument must be a **string literal** in Phase B. Expression-valued names (`smelt.config.var(other_var)`) are reserved for Phase E1.
- The return type is always `Text`.
- The lookup is resolved at type-check time, not at query runtime.

## Declaring variables in `smelt.yml`

Variables are declared in the `vars:` block of your workspace configuration:

```yaml
# examples/meta_hofs/smelt.yml
vars:
  region: us-west-2
  min_revenue: 100
  flag: true
  nullable: ~
```

## YAML scalar coercion rules

`smelt.config.var` renders every YAML scalar as `Text`. The coercion rules are:

| YAML value | Rendered `Text` | Notes |
|------------|-----------------|-------|
| String: `"hello"` | `'hello'` | Round-trips as-is |
| Boolean: `true` / `false` | `'true'` / `'false'` | |
| Integer: `42` | `'42'` | Decimal representation |
| Float: `3.14` | `'3.14'` | Decimal representation |
| Null: `~` or `null` | `''` | Coerces to empty string; emits `ConfigVarNullCoercion` warning |

Richer-typed reads (Boolean, Integer) land in Phase E1 with explicit schema declarations. In Phase B, cast the `Text` result yourself if you need a specific SQL type:

```sql
-- Cast the text result to an integer for comparison
WHERE revenue > CAST(smelt.config.var('min_revenue') AS INTEGER)
```

## Worked examples

**Example — reading a text variable:**

```sql
-- examples/meta_hofs/models/check_cols_with_config_var.sql
-- smelt.config.var('region') resolves to 'us-west-2' at compile time.
SELECT
    smelt.config.var('region'),
    smelt.config.var('min_revenue'),
    smelt.config.var('flag')
```

With `smelt.yml` declaring:
```yaml
vars:
  region: us-west-2
  min_revenue: 100
  flag: true
```

The compiler resolves:
- `smelt.config.var('region')` → `'us-west-2'`
- `smelt.config.var('min_revenue')` → `'100'` (integer coerced to text)
- `smelt.config.var('flag')` → `'true'` (boolean coerced to text)

**Example — null variable warning:**

```yaml
vars:
  nullable: ~   # YAML null
```

```sql
-- smelt.config.var('nullable') returns '' and warns ConfigVarNullCoercion
SELECT smelt.config.var('nullable')
```

**Example — conditional filtering with a config variable:**

```sql
-- Use a config variable as a filter threshold.
-- Cast from Text to the target SQL type explicitly in Phase B.
SELECT id, revenue
FROM smelt.sources.raw.orders
WHERE revenue > CAST(smelt.config.var('min_revenue') AS DECIMAL)
```

## LSP support

- **Hover** on `smelt.config.var('x')` shows the return type `Text` and the variable's resolved value when it is declared in `smelt.yml`.
- **Goto-definition** on the string literal argument resolves to the `vars.x:` line in `smelt.yml`.

## Diagnostic codes

---

!!! warning "ConfigVarNotFound"
    **When it fires:** `smelt.config.var('name')` is called but `name` is not present in the `vars:` block of `smelt.yml`.

    **Message:** `compile-time variable {name} not declared in smelt.yml vars`

    **Fires at:** the `smelt.config.var(...)` call site.

    **Example:**
    ```sql
    -- ← ConfigVarNotFound: 'threshold' is not in smelt.yml vars
    SELECT smelt.config.var('threshold')
    ```

    **What to fix:** Add the variable to the `vars:` block of your `smelt.yml`:
    ```yaml
    vars:
      threshold: 50
    ```
    If the variable is environment-specific, declare it in a per-target overlay (see the `smelt.yml` reference for target-level `vars:` overrides).

---

!!! warning "ConfigVarNameNotLiteral"
    **When it fires:** The argument to `smelt.config.var` is not a string literal.

    **Message:** `smelt.config.var name must be a string literal`

    **Fires at:** the argument expression.

    **Example:**
    ```sql
    -- ← ConfigVarNameNotLiteral: argument must be a literal, not a column reference
    SELECT smelt.config.var(var_name_column)
    ```

    **What to fix:** Replace the argument with a string literal: `smelt.config.var('the_variable_name')`. Dynamic variable-name resolution (computing the name at compile time from another meta value) is planned for Phase E1.

---

!!! note "ConfigVarNullCoercion (warning)"
    **When it fires:** A `vars:` entry has a YAML `null` value (`~` or `null`), which is coerced to an empty string `''`.

    **Message:** `null variable {name} coerced to empty string; declare a default in smelt.yml` (warning severity — the model still compiles)

    **Fires at:** the `smelt.config.var(...)` call site.

    **Example:**
    ```yaml
    # smelt.yml
    vars:
      nullable: ~   # null YAML value
    ```
    ```sql
    -- ConfigVarNullCoercion warning: 'nullable' coerces to ''
    SELECT smelt.config.var('nullable')
    ```

    **What to fix:** Replace the null value with a sensible default in `smelt.yml`:
    ```yaml
    vars:
      nullable: ''         # explicit empty string
      # or:
      nullable: default    # a real default value
    ```
    If null is intentional (the variable is optionally set per environment), suppress the warning by setting an explicit empty string as the fallback.
