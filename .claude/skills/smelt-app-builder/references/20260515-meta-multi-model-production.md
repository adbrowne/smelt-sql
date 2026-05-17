# Multi-model production (generator files)

When a smelt project needs to emit many structurally identical models driven by
a config list (e.g. one model per cohort, one model per tenant), use a
**generator file** instead of hand-authoring each model separately.

See `docs-site/docs/meta-language/generators.md` for the full surface and syntax.

## Generator file vs. multi-section file — when to use which

| Use a generator file (`generates: models`) | Use a multi-section file (`--- name: … ---`) |
|---|---|
| Model count comes from a config file (YAML/JSON list) | Model count is fixed and hand-authored |
| Body is a HOF pipeline (`|> map(fn c => ModelDef {…})`) | Body has independent SQL per section |
| Name/schema derived from data | Each model has bespoke SQL |

The key question: "Is the set of models statically enumerable by a human?"
If yes → multi-section. If it grows with config → generator.

## `generates: models` + `name:` mutual exclusivity

A file is **either** a bare model OR a generator — never both.

```sql
-- ERROR: GeneratesMixedWithBareModel
---
name: my_model
generates: models
---
SELECT 1
```

Remove `name:` from the frontmatter; the generator file has no fixed model name.
Its emitted model names come from each `ModelDef.name` field.

## The emitted-path includes the file stem

Given `models/cohorts.gen.sql` emitting `ModelDef { name: 'us_west', … }`:

- Emitted smelt path: **`cohorts.us_west`** (not `us_west`)
- Downstream reference: `smelt.cohorts.us_west`

The path is: `<dir_components>.<file_stem>.<model_name>`.

Multi-extension stems are collapsed to the first extension:
`cohorts.gen.sql` → stem `cohorts`, not `cohorts.gen`.

## `smelt.models.*` is forbidden inside generator bodies

`smelt.models.with_tag(…)` and `smelt.models.all()` **cannot be called inside
a generator file body**. This prevents circular reflection (a generator trying
to iterate its own emissions before they are finalised).

```sql
-- ERROR: GeneratorBodyForbidsModelReflection
---
generates: models
---
smelt.models.with_tag('cohort')  -- forbidden here
  |> map(fn m => ModelDef { name: m.name, body: SELECT 1 })
```

Use `smelt.sources.*` (source-table reflection), `smelt.config.load_yaml`, or
`smelt.config.var` to drive generation instead.

## The killer-demo pattern: YAML config → List<ModelDef> → downstream union

```yaml
# cohorts.yaml
- name: us_west
  region: us-west-2
  min_revenue: 100
- name: eu
  region: eu-west-1
  min_revenue: 50
```

```sql
-- models/cohorts.gen.sql
---
generates: models
tags: [cohort]
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text, region: Text, min_revenue: Integer }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT id, user_id, region, revenue
             FROM smelt.orders
             WHERE region = c.region AND revenue >= c.min_revenue
     })
```

```sql
-- models/all_cohorts.sql
SELECT id, user_id, region, revenue FROM smelt.cohorts.us_west
UNION ALL
SELECT id, user_id, region, revenue FROM smelt.cohorts.eu
```

Alternatively, use the `union_all` reducer on the tag-filtered model set to
avoid hardcoding the list:

```sql
-- models/all_cohorts.sql
smelt.models.with_tag('cohort') |> reduce(union_all)
```

## Schema-evolution-safe acceptance test pattern

The canonical acceptance test for a generator-driven union verifies that the
union's row count equals the sum of per-cohort filtered counts:

```sql
-- tests/cohort_count.test.sql
---
materialization: test
---
SELECT
    (SELECT COUNT(*) FROM smelt.all_cohorts)
    = (SELECT SUM(cnt) FROM (
        SELECT COUNT(*) AS cnt FROM smelt.orders WHERE region = 'us-west-2' AND revenue >= 100
        UNION ALL
        SELECT COUNT(*) AS cnt FROM smelt.orders WHERE region = 'eu-west-1' AND revenue >= 50
      ) AS sub) AS passes
```

This pattern is robust to column-set changes in the emitted models because it
counts rows rather than comparing column values. If a new cohort is added to
the YAML, the test fails with a clear row-count mismatch rather than a schema
error.

## LSP behaviour

- Hover on `generates: models` in frontmatter → shows `List<ModelDef>` + emitted model count.
- Hover on `ModelDef {` opening brace → shows the inferred emitted smelt path.
- Hover on the `name:` value → shows `Emitted as smelt.<path>`.
- Goto-definition on `smelt.cohorts.us_west` in a consumer → jumps to the `name: 'us_west'`
  value token in `cohorts.gen.sql`.
- Completion at `generates: ` → offers `models` (the only valid value).
- Completion inside `ModelDef { ` → offers the five fields (`name`, `body`,
  `materialization`, `tags`, `description`) with required fields first.

See `docs-site/docs/meta-language/generators.md` for the full reference.
