---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT cost, platform, transaction_id
    FROM smelt.ref('py_l3_362')
    WHERE status = 'active'
)
SELECT
    b.cost,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_362') j ON b.user_id = j.user_id
GROUP BY b.cost
