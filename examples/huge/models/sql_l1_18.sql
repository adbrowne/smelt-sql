---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT price, session_id, channel
    FROM smelt.payments
    WHERE platform = 'web'
),
aggregated AS (
    SELECT price, COUNT(*) AS cnt
    FROM filtered
    GROUP BY price
)
SELECT
    a.price,
    a.cnt,
    f.session_id
FROM aggregated a
INNER JOIN filtered f ON a.price = f.price
