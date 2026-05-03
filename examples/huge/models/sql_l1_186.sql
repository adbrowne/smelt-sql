---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.score,
    a.quantity,
    b.discount
FROM smelt.payments a
INNER JOIN smelt.payments b ON a.user_id = b.user_id

