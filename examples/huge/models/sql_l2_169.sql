---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    updated_at,
    created_at
FROM smelt.models.sql_l1_156
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_108 WHERE country = 'US'
)

