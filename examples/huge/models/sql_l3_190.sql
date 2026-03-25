---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_date, event_type, score
    FROM smelt.ref('py_l2_400')
    WHERE event_type = 'purchase'
)
SELECT
    b.event_date,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_95') j ON b.user_id = j.user_id
GROUP BY b.event_date
