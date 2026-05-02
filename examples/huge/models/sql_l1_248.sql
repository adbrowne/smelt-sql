---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.user_id,
    b.cohort_date,
    c.profit,
    c.duration_seconds
FROM smelt.models.campaigns a
INNER JOIN smelt.models.campaigns b ON a.user_id = b.user_id
LEFT JOIN smelt.models.campaigns c ON a.user_id = c.user_id

