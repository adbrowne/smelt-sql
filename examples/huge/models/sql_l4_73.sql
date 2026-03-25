---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, plan_type, amount, 'source_0' AS source_tag FROM smelt.ref('sql_l3_89')
UNION ALL
SELECT browser, plan_type, amount, 'source_1' AS source_tag FROM smelt.ref('sql_l3_192')
UNION ALL
SELECT browser, plan_type, amount, 'source_2' AS source_tag FROM smelt.ref('sql_l3_183')
