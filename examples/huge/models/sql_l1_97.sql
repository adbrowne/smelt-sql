---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    os_name,
    is_verified
FROM smelt.ref('categories')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('categories') WHERE category IS NOT NULL
)
