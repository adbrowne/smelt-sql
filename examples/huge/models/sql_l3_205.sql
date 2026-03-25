---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT duration_seconds, os_name, category, 'source_0' AS source_tag FROM smelt.ref('sql_l2_62')
UNION ALL
SELECT duration_seconds, os_name, category, 'source_1' AS source_tag FROM smelt.ref('sql_l2_92')
