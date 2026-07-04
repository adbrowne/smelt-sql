---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT price, referrer, duration_seconds
    FROM smelt.sql_l1_244
    WHERE platform = 'web'
)
SELECT
    b.price,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_182 j ON b.user_id = j.user_id
GROUP BY b.price
