---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT price, event_type, rating
    FROM smelt.sql_l3_235
    WHERE score >= 50
),
aggregated AS (
    SELECT price, COUNT(*) AS cnt
    FROM filtered
    GROUP BY price
)
SELECT
    a.price,
    a.cnt,
    f.event_type
FROM aggregated a
INNER JOIN filtered f ON a.price = f.price
