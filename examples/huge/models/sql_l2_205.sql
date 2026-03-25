---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT profit, cohort_date, transaction_id
    FROM smelt.ref('py_l1_411')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT profit, COUNT(*) AS cnt
    FROM filtered
    GROUP BY profit
)
SELECT
    a.profit,
    a.cnt,
    f.cohort_date
FROM aggregated a
INNER JOIN filtered f ON a.profit = f.profit
