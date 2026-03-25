---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    cohort_date,
    duration_seconds,
    device_type
FROM smelt.ref('sql_l3_170')
WHERE country = 'US'
