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
    SELECT session_id, browser, revenue
    FROM smelt.sql_l2_51
    WHERE quantity > 0
)
SELECT
    b.session_id,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_123 j ON b.user_id = j.user_id
GROUP BY b.session_id
