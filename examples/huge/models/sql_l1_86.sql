---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    campaign_id,
    cohort_date,
    status
FROM smelt.models.errors
WHERE quantity > 0

