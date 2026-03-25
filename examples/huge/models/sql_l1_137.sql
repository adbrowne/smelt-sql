---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT referrer, order_id, transaction_id
    FROM smelt.ref('signups')
    WHERE platform = 'web'
)
SELECT
    b.referrer,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('signups') j ON b.user_id = j.user_id
GROUP BY b.referrer
