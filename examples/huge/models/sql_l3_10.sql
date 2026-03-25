---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT category, status, os_name, 'source_0' AS source_tag FROM smelt.ref('sql_l2_96')
UNION ALL
SELECT category, status, os_name, 'source_1' AS source_tag FROM smelt.ref('py_l2_286')
UNION ALL
SELECT category, status, os_name, 'source_2' AS source_tag FROM smelt.ref('sql_l2_88')
