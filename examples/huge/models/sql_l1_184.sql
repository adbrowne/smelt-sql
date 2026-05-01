---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.browser,
    b.transaction_id,
    c.updated_at,
    c.revenue
FROM smelt.models.sessions a
INNER JOIN smelt.models.sessions b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sessions c ON a.user_id = c.user_id

