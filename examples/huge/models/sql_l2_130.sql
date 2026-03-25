---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_type, browser, session_id
    FROM smelt.ref('py_l1_321')
    WHERE amount > 0
)
SELECT
    b.event_type,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_168') j ON b.user_id = j.user_id
GROUP BY b.event_type
