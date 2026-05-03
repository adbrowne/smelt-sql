---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT country, campaign_id, is_active
    FROM smelt.sql_l1_167
    WHERE platform = 'web'
)
SELECT
    b.country,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_165 j ON b.user_id = j.user_id
GROUP BY b.country

