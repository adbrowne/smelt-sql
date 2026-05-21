---
materialization: table
incremental:
  enabled: true
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

