---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT platform, ip_address, category
    FROM smelt.ref('sql_l1_83')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.platform,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_218') j ON b.user_id = j.user_id
GROUP BY b.platform
