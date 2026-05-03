---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_time, campaign_id, session_id
    FROM smelt.sql_l3_118
    WHERE score >= 50
)
SELECT
    b.event_time,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_58 j ON b.user_id = j.user_id
GROUP BY b.event_time

