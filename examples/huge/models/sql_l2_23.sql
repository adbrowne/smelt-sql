---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT revenue, duration_seconds, created_at, 'source_0' AS source_tag FROM smelt.ref('sql_l1_35')
UNION ALL
SELECT revenue, duration_seconds, created_at, 'source_1' AS source_tag FROM smelt.ref('py_l1_331')
UNION ALL
SELECT revenue, duration_seconds, created_at, 'source_2' AS source_tag FROM smelt.ref('py_l1_314')
