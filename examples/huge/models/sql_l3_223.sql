---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT platform, browser, price
    FROM smelt.ref('sql_l2_57')
    WHERE status = 'active'
)
SELECT
    b.platform,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_108') j ON b.user_id = j.user_id
GROUP BY b.platform
