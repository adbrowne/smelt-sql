---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT rating, referrer, ip_address
    FROM smelt.ref('notifications')
    WHERE score >= 50
)
SELECT
    b.rating,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('notifications') j ON b.user_id = j.user_id
GROUP BY b.rating
