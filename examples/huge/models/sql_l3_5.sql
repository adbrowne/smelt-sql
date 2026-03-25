---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT region, status, country
    FROM smelt.ref('py_l2_294')
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT region, COUNT(*) AS cnt
    FROM filtered
    GROUP BY region
)
SELECT
    a.region,
    a.cnt,
    f.status
FROM aggregated a
INNER JOIN filtered f ON a.region = f.region
