---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT tier, rating, page_path
    FROM smelt.ref('logs')
    WHERE country = 'US'
)
SELECT
    b.tier,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('logs') j ON b.user_id = j.user_id
GROUP BY b.tier
