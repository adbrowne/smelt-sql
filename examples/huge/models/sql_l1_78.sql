---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT tier, created_at, device_type
    FROM smelt.ref('subscriptions')
    WHERE platform = 'web'
)
SELECT
    b.tier,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('subscriptions') j ON b.user_id = j.user_id
GROUP BY b.tier
