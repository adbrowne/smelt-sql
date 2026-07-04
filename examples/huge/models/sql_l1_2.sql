---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT user_id, revenue, email_domain
    FROM smelt.invoices
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.revenue
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
