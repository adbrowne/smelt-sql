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
    SELECT amount, created_at, discount
    FROM smelt.sql_l2_214
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
