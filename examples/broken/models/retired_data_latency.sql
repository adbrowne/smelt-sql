---
materialization: table
columns:
  order_date:
    data_latency: "3 days"
---
-- The per-column `data_latency:` key is retired (`docs/specs/models.md`
-- §Diagnostics `MalformedFrontmatter`/`YamlParseError`): declared lateness is
-- expressed once on the source as `mutation_profile.lateness`, never per
-- column.
SELECT order_date, customer_id
FROM smelt.sources.maintenance_orders
