---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT amount, cost, cohort_date
    FROM smelt.ref('py_l1_391')
    WHERE quantity > 0
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.cost
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount
