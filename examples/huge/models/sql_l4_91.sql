---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT plan_type, updated_at, browser, 'source_0' AS source_tag FROM smelt.ref('py_l3_344')
UNION ALL
SELECT plan_type, updated_at, browser, 'source_1' AS source_tag FROM smelt.ref('sql_l3_23')
UNION ALL
SELECT plan_type, updated_at, browser, 'source_2' AS source_tag FROM smelt.ref('py_l3_444')
