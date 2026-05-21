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
    profit,
    campaign_id,
    cohort_date,
    status
FROM smelt.errors
WHERE quantity > 0

