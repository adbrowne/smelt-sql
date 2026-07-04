---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT is_active, referrer, region, 'source_0' AS source_tag FROM smelt.sql_l3_80
UNION ALL
SELECT is_active, referrer, region, 'source_1' AS source_tag FROM smelt.sql_l3_20
