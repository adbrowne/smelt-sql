---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT profit, event_time, is_active, 'source_0' AS source_tag FROM smelt.ref('orders')
UNION ALL
SELECT profit, event_time, is_active, 'source_1' AS source_tag FROM smelt.ref('orders')
