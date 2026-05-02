---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.quantity,
    b.ip_address,
    c.score,
    c.segment
FROM smelt.models.clicks a
INNER JOIN smelt.models.clicks b ON a.user_id = b.user_id
LEFT JOIN smelt.models.clicks c ON a.user_id = c.user_id

