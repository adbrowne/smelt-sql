---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT transaction_id, email_domain, os_name
    FROM smelt.sql_l1_177
    WHERE amount > 0
),
aggregated AS (
    SELECT transaction_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY transaction_id
)
SELECT
    a.transaction_id,
    a.cnt,
    f.email_domain
FROM aggregated a
INNER JOIN filtered f ON a.transaction_id = f.transaction_id
