---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT cohort_date, profit, event_time
    FROM smelt.ref('sql_l2_246')
    WHERE is_active = true
)
SELECT
    b.cohort_date,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_421') j ON b.user_id = j.user_id
GROUP BY b.cohort_date
