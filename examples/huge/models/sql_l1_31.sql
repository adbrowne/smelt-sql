---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.revenue,
    b.updated_at,
    c.cohort_date,
    c.order_id
FROM smelt.ref('sessions') a
INNER JOIN smelt.ref('sessions') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sessions') c ON a.user_id = c.user_id
