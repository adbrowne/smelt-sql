---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_time, score, plan_type
    FROM smelt.ref('py_l2_284')
    WHERE category IS NOT NULL
)
SELECT
    b.event_time,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_141') j ON b.user_id = j.user_id
GROUP BY b.event_time
