---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT revenue, plan_type, region
    FROM smelt.ref('orders')
    WHERE status = 'active'
),
aggregated AS (
    SELECT revenue, COUNT(*) AS cnt
    FROM filtered
    GROUP BY revenue
)
SELECT
    a.revenue,
    a.cnt,
    f.plan_type
FROM aggregated a
INNER JOIN filtered f ON a.revenue = f.revenue
