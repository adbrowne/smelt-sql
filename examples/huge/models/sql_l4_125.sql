---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT tier, revenue, event_date
    FROM smelt.ref('sql_l3_188')
    WHERE platform = 'web'
)
SELECT
    b.tier,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_281') j ON b.user_id = j.user_id
GROUP BY b.tier
