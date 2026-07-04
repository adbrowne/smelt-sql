---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_type, browser, session_id
    FROM smelt.sql_l1_115
    WHERE amount > 0
)
SELECT
    b.event_type,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_115 j ON b.user_id = j.user_id
GROUP BY b.event_type
