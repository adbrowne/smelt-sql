---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT profit, event_time, is_active, 'source_0' AS source_tag FROM smelt.orders
UNION ALL
SELECT profit, event_time, is_active, 'source_1' AS source_tag FROM smelt.orders

