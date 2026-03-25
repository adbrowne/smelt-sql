---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    cohort_date,
    created_at,
    channel
FROM smelt.ref('sql_l3_111')
WHERE country = 'US'
