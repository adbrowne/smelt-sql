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
    event_type,
    duration_seconds,
    cohort_date,
    segment
FROM smelt.campaigns
WHERE quantity > 0

