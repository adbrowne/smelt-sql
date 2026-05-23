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
    SELECT segment, product_id, email_domain
    FROM smelt.page_views
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.segment,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.page_views j ON b.user_id = j.user_id
GROUP BY b.segment
