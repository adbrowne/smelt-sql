# Generator Files: Multi-Model Production

A **generator file** produces multiple models from a single compile-time expression. Instead of writing one `.sql` file per model, you write a meta-language expression that evaluates to a `List<ModelDef>` — one entry per model to emit.

## Declaring a generator file

Add `generates: models` to the YAML frontmatter:

```sql
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

The file body is a meta-language expression of type `List<ModelDef>`. Each `ModelDef` value in the list becomes a model in the workspace.

## ModelDef — the closed five-field record

`ModelDef` is a built-in closed record type. It is user-constructible only inside a generator file body.

| Field | Type | Required | Default |
|-------|------|----------|---------|
| `name` | `Text` | Yes | — |
| `body` | `TableExpr` | Yes | — |
| `materialization` | `Text` | No | `"view"` |
| `tags` | `List<Text>` | No | `[]` |
| `description` | `Text` | No | `""` |

The `name` field must be a path-safe identifier (ASCII letters, digits, underscores only; must not start with a digit).

## Emitted model paths

Each emitted `ModelDef` with `name: 'n'` from a generator at workspace-relative path `<dir>/<base>.gen.sql` becomes:

```
smelt.<dir>.<base>.<n>
```

For example, `models/cohorts.gen.sql` emitting `name: 'us_west'` → `smelt.cohorts.us_west`.

Generator-emitted models are referenced the same way as hand-authored models:

```sql
-- Downstream model consuming an emitted model:
SELECT * FROM smelt.cohorts.us_west
```

## Frontmatter inheritance

A generator file's frontmatter is inherited by all emitted models:

- `tags`: frontmatter tags are merged with per-`ModelDef` tags.
- `materialization`: the frontmatter value is the default; each `ModelDef` may override it via the `materialization` field.
- `incremental:` block: shared by all incremental emissions from the same generator. Per-`ModelDef` overrides are not supported in v1.

## Name uniqueness and collision rules

- **Per-file duplicates.** Two `ModelDef` values with the same `name` in the same generator emit `ModelDefDuplicateName` at the second occurrence. The first is retained; the second is discarded.
- **Hand-authored wins.** If an emitted model's smelt path matches a hand-authored model's path, `ModelDefHandAuthoredCollision` is emitted at the offending `name` field. The hand-authored model is retained.
- **Cross-generator collision.** Two generators emitting the same smelt path emit `ModelDefHandAuthoredCollision` on the second generator (by workspace-relative path order). The first generator's emission is retained.

## Restrictions

- A `ModelDef` literal outside a generator file body emits `ModelDefOutsideGeneratorFile`.
- A generator file containing a top-level bare `SELECT` / `WITH` / `VALUES` statement emits `GenerateFileBareSelectForbidden`.
- A generator body that does not evaluate to `List<ModelDef>` emits `GenerateFileBodyTypeError`.

### Generator-body reflection restriction

A generator body **must not** call `smelt.models.with_tag` or `smelt.models.all`. Doing so emits `GeneratorBodyForbidsModelReflection`.

**Why.** Workspace shape (which models exist) is determined by evaluating all generators; admitting `smelt.models.*` inside a generator would create a circular dependency between generator emissions and the model-reflection they observe.

**Alternative.** Use `smelt.sources.*` or loaders to drive the generation:

```sql
-- OK: sources are loader-time, evaluated before generators.
[
  ModelDef { name: 'orders', body: SELECT * FROM smelt.sources.raw.orders },
  ModelDef { name: 'users',  body: SELECT * FROM smelt.sources.raw.users }
]

-- Also OK: literal smelt.<path> references to hand-authored models.
ModelDef {
  name: 'summary',
  body: SELECT * FROM smelt.staging.orders WHERE status = 'complete'
}
```

## Complete example: per-cohort union

`cohorts.yaml`:
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

`models/cohorts.gen.sql`:
```sql
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

This emits three models: `smelt.cohorts.us_west`, `smelt.cohorts.us_east`, `smelt.cohorts.eu`.

`models/all_cohorts_unioned.sql` then references them:
```sql
SELECT * FROM smelt.cohorts.us_west
UNION ALL
SELECT * FROM smelt.cohorts.us_east
UNION ALL
SELECT * FROM smelt.cohorts.eu
```

The full working example is in `examples/per_cohort_union/`.

## Diagnostic codes

| Code | Trigger |
|------|---------|
| `GeneratesUnknownValue` | `generates:` value is not `models` |
| `GeneratesMixedWithBareModel` | `generates: models` combined with `name:` or Layer-1 delimiters |
| `GenerateFileBareSelectForbidden` | Bare `SELECT`/`WITH`/`VALUES` in a generator file body |
| `GenerateFileBodyTypeError` | Generator body type is not `List<ModelDef>` |
| `ModelDefOutsideGeneratorFile` | `ModelDef {…}` literal outside a generator file |
| `ModelDefInvalidName` | `name` field value is not a valid path-safe identifier |
| `ModelDefInvalidMaterialization` | `materialization` field value is not a known strategy |
| `ModelDefDuplicateName` | Two `ModelDef`s in the same file share the same `name` |
| `ModelDefHandAuthoredCollision` | Generator-emitted path collides with a hand-authored model or another generator's emission |
| `GeneratorBodyForbidsModelReflection` | Generator body calls `smelt.models.with_tag` or `smelt.models.all` |
