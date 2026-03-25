---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT tier, cost, created_at, 'source_0' AS source_tag FROM smelt.ref('py_l1_330')
UNION ALL
SELECT tier, cost, created_at, 'source_1' AS source_tag FROM smelt.ref('py_l1_406')
UNION ALL
SELECT tier, cost, created_at, 'source_2' AS source_tag FROM smelt.ref('py_l1_314')
