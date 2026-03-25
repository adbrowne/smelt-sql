---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_time, campaign_id, session_id
    FROM smelt.ref('sql_l3_32')
    WHERE score >= 50
)
SELECT
    b.event_time,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_231') j ON b.user_id = j.user_id
GROUP BY b.event_time
