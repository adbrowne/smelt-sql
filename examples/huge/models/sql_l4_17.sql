---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT discount, user_id, platform, 'source_0' AS source_tag FROM smelt.sql_l3_192
UNION ALL
SELECT discount, user_id, platform, 'source_1' AS source_tag FROM smelt.sql_l3_192
