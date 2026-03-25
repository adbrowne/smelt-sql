---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT updated_at, cost, os_name, 'source_0' AS source_tag FROM smelt.ref('py_l1_354')
UNION ALL
SELECT updated_at, cost, os_name, 'source_1' AS source_tag FROM smelt.ref('sql_l1_36')
