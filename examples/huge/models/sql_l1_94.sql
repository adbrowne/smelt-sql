---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT status, revenue, browser
    FROM smelt.ref('events')
    WHERE event_type = 'purchase'
)
SELECT
    b.status,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('events') j ON b.user_id = j.user_id
GROUP BY b.status
