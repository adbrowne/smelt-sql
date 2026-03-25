---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT is_verified, profit, campaign_id
    FROM smelt.ref('sql_l2_76')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.is_verified,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_455') j ON b.user_id = j.user_id
GROUP BY b.is_verified
