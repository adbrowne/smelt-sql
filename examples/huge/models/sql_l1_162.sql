---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT region, revenue, platform
    FROM smelt.ref('page_views')
    WHERE event_type = 'purchase'
)
SELECT
    b.region,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('page_views') j ON b.user_id = j.user_id
GROUP BY b.region
