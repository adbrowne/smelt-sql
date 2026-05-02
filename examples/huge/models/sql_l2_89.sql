---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT amount, transaction_id, device_type
    FROM smelt.models.sql_l1_123
    WHERE amount > 0
)
SELECT
    b.amount,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l1_53 j ON b.user_id = j.user_id
GROUP BY b.amount

