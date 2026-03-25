---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT revenue, event_date, cost
    FROM smelt.ref('py_l1_328')
    WHERE amount > 0
),
aggregated AS (
    SELECT revenue, COUNT(*) AS cnt
    FROM filtered
    GROUP BY revenue
)
SELECT
    a.revenue,
    a.cnt,
    f.event_date
FROM aggregated a
INNER JOIN filtered f ON a.revenue = f.revenue
