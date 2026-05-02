---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT transaction_id, user_id, price
    FROM smelt.models.users
    WHERE amount > 0
)
SELECT
    b.transaction_id,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.models.users j ON b.user_id = j.user_id
GROUP BY b.transaction_id

