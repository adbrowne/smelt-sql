---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT product_id, platform, duration_seconds, 'source_0' AS source_tag FROM smelt.sql_l3_13
UNION ALL
SELECT product_id, platform, duration_seconds, 'source_1' AS source_tag FROM smelt.sql_l3_163
