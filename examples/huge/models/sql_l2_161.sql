---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT quantity, os_name, price
    FROM smelt.sql_l1_182
    WHERE status = 'active'
),
aggregated AS (
    SELECT quantity, COUNT(*) AS cnt
    FROM filtered
    GROUP BY quantity
)
SELECT
    a.quantity,
    a.cnt,
    f.os_name
FROM aggregated a
INNER JOIN filtered f ON a.quantity = f.quantity
