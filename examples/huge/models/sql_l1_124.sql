---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT updated_at, event_date, duration_seconds
    FROM smelt.ref('notifications')
    WHERE event_type = 'purchase'
)
SELECT
    b.updated_at,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('notifications') j ON b.user_id = j.user_id
GROUP BY b.updated_at
