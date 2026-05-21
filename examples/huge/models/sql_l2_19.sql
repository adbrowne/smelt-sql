---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT amount, cost, cohort_date
    FROM smelt.sql_l1_0
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

