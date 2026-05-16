# Large fixture: generator-driven per-country cohorts

You are building a smelt data project that uses **generator files** to produce
one model per cohort, then unions them in a downstream model. This exercise
covers the Phase E2 multi-model production surface.

## Inputs

One seed CSV is already copied into `seeds/`:

- `seeds/raw_orders.csv` — 12 rows: `order_id`, `customer_id`, `order_date`,
  `country`, `status`, `amount`

`country` is one of `US`, `UK`, `DE`. `status` is `shipped` or `cancelled`.
`amount` is in dollars (decimal).

## Required project structure

### 1. YAML config listing three cohorts

Create `configs/cohorts.yaml` with exactly three entries, one per country:

```yaml
- name: us
  country: US
- name: uk
  country: UK
- name: de
  country: DE
```

The `name` field becomes the emitted model's suffix in the smelt path. You may
choose any valid identifier (lowercase, no spaces).

### 2. Generator file

Create `models/cohorts.gen.sql` with `generates: models` frontmatter. Its body
must use `smelt.config.load_yaml` to load `configs/cohorts.yaml` and a
`|> map(fn c => ModelDef {…})` pipeline to emit one model per cohort.

Each emitted model should SELECT only **shipped** orders for that country from
the seed table `smelt.raw_orders`:

```sql
---
generates: models
tags: [cohort]
---
smelt.config.load_yaml('configs/cohorts.yaml', List<{ name: Text, country: Text }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT order_id, customer_id, order_date, country, amount
             FROM smelt.raw_orders
             WHERE country = c.country AND status = 'shipped'
     })
```

The generator emits three models: one per cohort, filtered to shipped orders.

### 3. Downstream union model

Create `models/all_orders.sql` that unions all three emitted cohort models into
a single table. Reference each emitted model by its smelt path
(`smelt.cohorts.<name>`):

```sql
SELECT order_id, customer_id, order_date, country, amount FROM smelt.cohorts.us
UNION ALL
SELECT order_id, customer_id, order_date, country, amount FROM smelt.cohorts.uk
UNION ALL
SELECT order_id, customer_id, order_date, country, amount FROM smelt.cohorts.de
```

Tag this model with `materialization: table` so it is materialised and
inspectable.

### 4. Acceptance test

Create `tests/cohort_count.test.sql` with `materialization: test`. The test
must assert that the union's row count equals the sum of per-cohort shipped
order counts:

```sql
---
materialization: test
---
SELECT
    (SELECT COUNT(*) FROM smelt.all_orders)
    = (SELECT SUM(cnt) FROM (
        SELECT COUNT(*) AS cnt FROM smelt.raw_orders WHERE country = 'US' AND status = 'shipped'
        UNION ALL
        SELECT COUNT(*) AS cnt FROM smelt.raw_orders WHERE country = 'UK' AND status = 'shipped'
        UNION ALL
        SELECT COUNT(*) AS cnt FROM smelt.raw_orders WHERE country = 'DE' AND status = 'shipped'
      ) AS sub) AS passes
```

## `smelt.yml` requirements

Your `smelt.yml` must:
- List `models` and `tests` under `paths:` so the generator file and test are discovered.
- List `seeds` under `paths:` so `smelt.raw_orders` is available (or under a
  `seeds:` section if the CLI version supports it — check `smelt docs show getting-started/quickstart`).
- Set `default_materialization: view`.

## Required outputs

After `smelt build`, the DuckDB file must contain:

### `all_orders`
The union of all three cohort models — shipped orders from all countries.
Expected row count: **9** (3 US shipped + 3 UK shipped + 3 DE shipped).

### `cohorts_us`, `cohorts_uk`, `cohorts_de` (or equivalent names)
Individual per-cohort tables, each with 3 rows (the shipped orders for that
country).

Note: the exact table name in DuckDB depends on how smelt maps the emitted smelt
path to a table name. Check `smelt build --show-plan models/cohorts.gen.sql` to
see the emitted model names.

## How to know you are done

The harness runs `validate.py` automatically. To self-check:

```bash
duckdb my-project.duckdb -c "SELECT COUNT(*) FROM all_orders"
# Expected: 9
```

## Hints

- Read the skill at `<run_dir>/skill.md` and `smelt docs show meta-language/generators`
  before writing any code.
- Generator files must not reference `smelt.models.*` inside the body — use
  `smelt.sources.*` or loaders for generation inputs.
- The emitted smelt path for a cohort named `'us'` in `models/cohorts.gen.sql`
  is `smelt.cohorts.us`, NOT `smelt.us`. The path includes the file stem.
- Run `smelt build --show-plan models/cohorts.gen.sql` to confirm the generator
  expands correctly before building the full workspace.
- If the generator emits no models (empty pipeline), check that the YAML schema
  in `load_yaml` matches the actual keys in `cohorts.yaml`.
- The `tests/` directory must be listed under `paths:` in `smelt.yml` for the
  test model to be discovered and run.

## Optional: use `smelt.models.with_tag` for the union (Phase D surface)

Once the required outputs pass, try replacing the explicit
`smelt.cohorts.us UNION ALL …` union with the tag-driven reducer:

```sql
-- models/all_orders.sql
smelt.models.with_tag('cohort') |> reduce(union_all)
```

This makes `all_orders` automatically include any new cohort added to
`cohorts.yaml`, without editing the union model. The `validate.py` acceptance
check passes regardless of which union form you use — it only checks row counts.
