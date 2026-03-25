---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_type, session_id, is_verified
    FROM smelt.ref('sql_l1_80')
    WHERE event_type = 'purchase'
)
SELECT
    b.event_type,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_80') j ON b.user_id = j.user_id
GROUP BY b.event_type
