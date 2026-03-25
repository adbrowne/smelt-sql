---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT segment, cost, country
    FROM smelt.ref('transactions')
    WHERE platform = 'web'
)
SELECT
    b.segment,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('transactions') j ON b.user_id = j.user_id
GROUP BY b.segment
