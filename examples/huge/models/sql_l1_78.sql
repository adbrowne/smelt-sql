---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT tier, created_at, device_type
    FROM smelt.models.subscriptions
    WHERE platform = 'web'
)
SELECT
    b.tier,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.models.subscriptions j ON b.user_id = j.user_id
GROUP BY b.tier

