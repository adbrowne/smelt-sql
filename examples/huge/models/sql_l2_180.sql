---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT is_active, tier, session_id
    FROM smelt.ref('sql_l1_46')
    WHERE category IS NOT NULL
)
SELECT
    b.is_active,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_46') j ON b.user_id = j.user_id
GROUP BY b.is_active
