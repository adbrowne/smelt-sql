---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT device_type, event_type, quantity
    FROM smelt.sql_l1_10
    WHERE country = 'US'
)
SELECT
    b.device_type,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_10 j ON b.user_id = j.user_id
GROUP BY b.device_type
