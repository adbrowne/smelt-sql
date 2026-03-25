---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT plan_type, quantity, price, 'source_0' AS source_tag FROM smelt.ref('sql_l2_96')
UNION ALL
SELECT plan_type, quantity, price, 'source_1' AS source_tag FROM smelt.ref('py_l2_471')
UNION ALL
SELECT plan_type, quantity, price, 'source_2' AS source_tag FROM smelt.ref('sql_l2_144')
