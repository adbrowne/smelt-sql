---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT duration_seconds, quantity, status, 'source_0' AS source_tag FROM smelt.ref('sql_l1_76')
UNION ALL
SELECT duration_seconds, quantity, status, 'source_1' AS source_tag FROM smelt.ref('sql_l1_76')
