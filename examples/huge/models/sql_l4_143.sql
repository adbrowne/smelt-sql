---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT price, duration_seconds, discount, 'source_0' AS source_tag FROM smelt.sql_l3_34
UNION ALL
SELECT price, duration_seconds, discount, 'source_1' AS source_tag FROM smelt.sql_l3_214
