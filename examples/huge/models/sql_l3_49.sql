---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT cost, is_active, page_path
    FROM smelt.sql_l2_211
    WHERE amount > 0
)
SELECT
    b.cost,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_103 j ON b.user_id = j.user_id
GROUP BY b.cost
