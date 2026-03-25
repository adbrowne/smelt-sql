---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT amount, created_at, discount
    FROM smelt.ref('py_l2_271')
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.created_at
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount
