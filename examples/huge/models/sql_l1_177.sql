---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.rating,
    b.event_time,
    c.is_active,
    c.revenue
FROM smelt.ref('campaigns') a
INNER JOIN smelt.ref('campaigns') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('campaigns') c ON a.user_id = c.user_id
