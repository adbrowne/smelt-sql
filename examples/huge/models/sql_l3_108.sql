---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_type, campaign_id, platform
    FROM smelt.models.sql_l2_115
    WHERE is_active = true
)
SELECT
    b.event_type,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l2_61 j ON b.user_id = j.user_id
GROUP BY b.event_type

