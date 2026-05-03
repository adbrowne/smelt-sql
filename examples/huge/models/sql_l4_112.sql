---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT platform, rating, price
    FROM smelt.sql_l3_225
    WHERE platform = 'web'
)
SELECT
    b.platform,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_225 j ON b.user_id = j.user_id
GROUP BY b.platform

