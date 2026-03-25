---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT is_active, tier, session_id
    FROM smelt.ref('py_l1_424')
    WHERE category IS NOT NULL
)
SELECT
    b.is_active,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_253') j ON b.user_id = j.user_id
GROUP BY b.is_active
