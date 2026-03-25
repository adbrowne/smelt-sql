---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT profit, rating, ip_address
    FROM smelt.ref('errors')
    WHERE platform = 'web'
)
SELECT
    b.profit,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('errors') j ON b.user_id = j.user_id
GROUP BY b.profit
