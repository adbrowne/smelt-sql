---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT amount, transaction_id, device_type
    FROM smelt.sql_l1_123
    WHERE amount > 0
)
SELECT
    b.amount,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_53 j ON b.user_id = j.user_id
GROUP BY b.amount
