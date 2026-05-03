---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT profit, rating, ip_address
    FROM smelt.errors
    WHERE platform = 'web'
)
SELECT
    b.profit,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.errors j ON b.user_id = j.user_id
GROUP BY b.profit

