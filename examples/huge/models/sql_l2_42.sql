---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT revenue, event_date, cost
    FROM smelt.sql_l1_193
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

