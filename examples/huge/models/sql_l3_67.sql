---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_type, transaction_id, referrer
    FROM smelt.sql_l2_95
    WHERE quantity > 0
)
SELECT
    b.event_type,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_122 j ON b.user_id = j.user_id
GROUP BY b.event_type
