---
generates: models
---
smelt.config.load_yaml('configs/tenants.yaml', List<{ name: Text, schema: Text, threshold: Integer }>)
  |> map(fn t => ModelDef {
       name: t.name,
       body: SELECT 1 AS sentinel,
                    t.threshold AS tenant_threshold
     })
