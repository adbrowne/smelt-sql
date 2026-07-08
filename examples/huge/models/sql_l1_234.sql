---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT is_verified, created_at, user_id, 'source_0' AS source_tag FROM smelt.events
UNION ALL
SELECT is_verified, created_at, user_id, 'source_1' AS source_tag FROM smelt.events
