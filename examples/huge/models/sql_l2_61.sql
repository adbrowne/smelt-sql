---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT plan_type, channel, revenue
    FROM smelt.ref('py_l1_323')
    WHERE is_active = true
),
aggregated AS (
    SELECT plan_type, COUNT(*) AS cnt
    FROM filtered
    GROUP BY plan_type
)
SELECT
    a.plan_type,
    a.cnt,
    f.channel
FROM aggregated a
INNER JOIN filtered f ON a.plan_type = f.plan_type
