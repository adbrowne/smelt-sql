---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    cohort_date,
    category,
    referrer
FROM smelt.models.orders
WHERE is_active = true

