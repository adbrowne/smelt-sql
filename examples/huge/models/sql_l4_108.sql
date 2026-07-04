---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_time, transaction_id, product_id
    FROM smelt.sql_l3_242
    WHERE amount > 0
)
SELECT
    b.event_time,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_242 j ON b.user_id = j.user_id
GROUP BY b.event_time
