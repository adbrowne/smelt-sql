---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_type, campaign_id, platform
    FROM smelt.ref('sql_l2_15')
    WHERE is_active = true
)
SELECT
    b.event_type,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_115') j ON b.user_id = j.user_id
GROUP BY b.event_type
