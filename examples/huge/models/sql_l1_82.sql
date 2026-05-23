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
    SELECT campaign_id, channel, os_name
    FROM smelt.categories
    WHERE category IS NOT NULL
)
SELECT
    b.campaign_id,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.categories j ON b.user_id = j.user_id
GROUP BY b.campaign_id
