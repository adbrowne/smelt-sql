---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_type, session_id, is_verified
    FROM smelt.sql_l1_51
    WHERE event_type = 'purchase'
)
SELECT
    b.event_type,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_70 j ON b.user_id = j.user_id
GROUP BY b.event_type

