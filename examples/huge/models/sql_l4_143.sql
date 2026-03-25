---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT price, duration_seconds, discount, 'source_0' AS source_tag FROM smelt.ref('sql_l3_238')
UNION ALL
SELECT price, duration_seconds, discount, 'source_1' AS source_tag FROM smelt.ref('sql_l3_238')
