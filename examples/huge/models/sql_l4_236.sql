---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT category, is_active, session_id
    FROM smelt.sql_l3_240
    WHERE event_type = 'purchase'
)
SELECT
    b.category,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_234 j ON b.user_id = j.user_id
GROUP BY b.category

