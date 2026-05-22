---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT rating, user_id, price, 'source_0' AS source_tag FROM smelt.users
UNION ALL
SELECT rating, user_id, price, 'source_1' AS source_tag FROM smelt.users
