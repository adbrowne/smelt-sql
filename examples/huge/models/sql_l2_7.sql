---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT cost, page_path, platform
    FROM smelt.ref('py_l1_457')
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT cost, COUNT(*) AS cnt
    FROM filtered
    GROUP BY cost
)
SELECT
    a.cost,
    a.cnt,
    f.page_path
FROM aggregated a
INNER JOIN filtered f ON a.cost = f.cost
