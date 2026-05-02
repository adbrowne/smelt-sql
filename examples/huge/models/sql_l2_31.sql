---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT browser, channel, user_id
    FROM smelt.models.sql_l1_80
    WHERE event_type = 'purchase'
)
SELECT
    b.browser,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l1_135 j ON b.user_id = j.user_id
GROUP BY b.browser

