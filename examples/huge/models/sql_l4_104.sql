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
    SELECT user_id, segment, order_id
    FROM smelt.sql_l3_247
    WHERE amount > 0
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.segment
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id

