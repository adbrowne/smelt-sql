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
