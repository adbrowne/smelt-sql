---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT quantity, ip_address, rating
    FROM smelt.ref('events')
    WHERE is_active = true
)
SELECT
    b.quantity,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('events') j ON b.user_id = j.user_id
GROUP BY b.quantity
