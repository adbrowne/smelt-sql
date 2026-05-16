# staging_from_sources

Example workspace demonstrating generator files that produce one staging model per raw source.

## What this shows

`models/staging/staging.gen.sql` is a generator file (`generates: models`) that emits one
`ModelDef` per raw source — `orders`, `users`, and `products`. Each emitted model selects all
columns from the corresponding `smelt.sources.raw.*` table.

The generator body is currently written as a hand-authored list literal:

```sql
---
generates: models
---
[
  ModelDef { name: 'orders',   body: SELECT … FROM smelt.sources.raw.orders },
  ModelDef { name: 'users',    body: SELECT … FROM smelt.sources.raw.users },
  ModelDef { name: 'products', body: SELECT … FROM smelt.sources.raw.products }
]
```

## Known divergence: `smelt.sources.with_tag` as a generator-body driver

The intended idiomatic form of this generator is:

```sql
---
generates: models
tags: [staging]
---
smelt.sources.with_tag('raw')
  |> map(fn s => ModelDef {
       name: s.name,
       body: SELECT * FROM s
     })
```

This form is **valid at the type-system level** — `smelt.sources.with_tag` is admitted inside
generator bodies (unlike `smelt.models.*`, which is forbidden). However, the runtime evaluator
(`evaluate_body_emissions` in `smelt-db`) currently only enumerates models when the pipeline
driver is a `smelt.config.load_yaml` or `smelt.config.load_json` call. A
`smelt.sources.with_tag` driver resolves to zero emissions at evaluation time.

Until the evaluator is extended to handle source-reflection drivers, this fixture uses the
equivalent hardcoded list. The tracking plan is
[`docs/plans/20260509-meta-language-E2.md`](../../docs/plans/20260509-meta-language-E2.md).
