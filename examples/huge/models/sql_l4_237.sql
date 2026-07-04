---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT tier, quantity, order_id, 'source_0' AS source_tag FROM smelt.sql_l3_48
UNION ALL
SELECT tier, quantity, order_id, 'source_1' AS source_tag FROM smelt.sql_l3_48
