---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT cost, os_name, ip_address
    FROM smelt.models.clicks
    WHERE score >= 50
),
aggregated AS (
    SELECT cost, COUNT(*) AS cnt
    FROM filtered
    GROUP BY cost
)
SELECT
    a.cost,
    a.cnt,
    f.os_name
FROM aggregated a
INNER JOIN filtered f ON a.cost = f.cost

