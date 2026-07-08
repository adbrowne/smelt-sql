---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    country,
    event_type,
    plan_type
FROM smelt.sessions
WHERE score >= 50
