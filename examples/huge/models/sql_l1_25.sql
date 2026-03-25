---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT cost, os_name, ip_address
    FROM smelt.ref('clicks')
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
