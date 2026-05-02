---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT region, revenue, platform
    FROM smelt.models.page_views
    WHERE event_type = 'purchase'
)
SELECT
    b.region,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.models.page_views j ON b.user_id = j.user_id
GROUP BY b.region

