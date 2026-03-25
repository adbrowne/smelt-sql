---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.product_id,
    b.score,
    c.duration_seconds,
    c.country
FROM smelt.ref('clicks') a
INNER JOIN smelt.ref('clicks') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('clicks') c ON a.user_id = c.user_id
