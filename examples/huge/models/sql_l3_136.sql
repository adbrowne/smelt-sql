---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT tier, platform, revenue
    FROM smelt.sql_l2_1
    WHERE platform = 'web'
),
aggregated AS (
    SELECT tier, COUNT(*) AS cnt
    FROM filtered
    GROUP BY tier
)
SELECT
    a.tier,
    a.cnt,
    f.platform
FROM aggregated a
INNER JOIN filtered f ON a.tier = f.tier

