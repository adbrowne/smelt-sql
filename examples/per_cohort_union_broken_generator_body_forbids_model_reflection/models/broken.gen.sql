---
generates: models
---
smelt.models.with_tag('cohort')
  |> map(fn m => ModelDef { name: m.name, body: SELECT 1 })
