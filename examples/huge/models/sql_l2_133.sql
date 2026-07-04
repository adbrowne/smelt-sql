---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT transaction_id, platform, is_verified
    FROM smelt.sql_l1_174
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT transaction_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY transaction_id
)
SELECT
    a.transaction_id,
    a.cnt,
    f.platform
FROM aggregated a
INNER JOIN filtered f ON a.transaction_id = f.transaction_id
