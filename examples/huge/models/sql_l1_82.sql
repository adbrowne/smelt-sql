---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT campaign_id, channel, os_name
    FROM smelt.ref('categories')
    WHERE category IS NOT NULL
)
SELECT
    b.campaign_id,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('categories') j ON b.user_id = j.user_id
GROUP BY b.campaign_id
