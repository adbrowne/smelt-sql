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
    SELECT transaction_id, user_id, price
    FROM smelt.users
    WHERE amount > 0
)
SELECT
    b.transaction_id,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.users j ON b.user_id = j.user_id
GROUP BY b.transaction_id
