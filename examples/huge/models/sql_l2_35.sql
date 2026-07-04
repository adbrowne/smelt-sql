---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT category, profit, product_id
    FROM smelt.sql_l1_54
    WHERE is_active = true
),
aggregated AS (
    SELECT category, COUNT(*) AS cnt
    FROM filtered
    GROUP BY category
)
SELECT
    a.category,
    a.cnt,
    f.profit
FROM aggregated a
INNER JOIN filtered f ON a.category = f.category
