---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_type, is_active, event_time
    FROM smelt.ref('transactions')
    WHERE quantity > 0
)
SELECT
    b.event_type,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('transactions') j ON b.user_id = j.user_id
GROUP BY b.event_type
