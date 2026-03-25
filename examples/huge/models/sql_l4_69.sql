---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT is_active, referrer, region, 'source_0' AS source_tag FROM smelt.ref('py_l3_391')
UNION ALL
SELECT is_active, referrer, region, 'source_1' AS source_tag FROM smelt.ref('sql_l3_115')
