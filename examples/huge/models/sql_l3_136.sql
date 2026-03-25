---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT tier, platform, revenue
    FROM smelt.ref('py_l2_272')
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
