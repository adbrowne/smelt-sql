---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT discount, session_id, page_path
    FROM smelt.sql_l2_122
    WHERE status = 'active'
)
SELECT
    b.discount,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_108 j ON b.user_id = j.user_id
GROUP BY b.discount
