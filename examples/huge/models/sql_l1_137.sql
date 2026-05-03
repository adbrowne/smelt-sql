---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT referrer, order_id, transaction_id
    FROM smelt.signups
    WHERE platform = 'web'
)
SELECT
    b.referrer,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.signups j ON b.user_id = j.user_id
GROUP BY b.referrer

