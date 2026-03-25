---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT amount, category, referrer
    FROM smelt.ref('orders')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.category
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount
