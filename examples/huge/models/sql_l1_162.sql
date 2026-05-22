---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT region, revenue, platform
    FROM smelt.page_views
    WHERE event_type = 'purchase'
)
SELECT
    b.region,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.page_views j ON b.user_id = j.user_id
GROUP BY b.region
