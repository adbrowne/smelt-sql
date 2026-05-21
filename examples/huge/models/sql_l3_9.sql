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
    SELECT email_domain, updated_at, score
    FROM smelt.sql_l2_23
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT email_domain, COUNT(*) AS cnt
    FROM filtered
    GROUP BY email_domain
)
SELECT
    a.email_domain,
    a.cnt,
    f.updated_at
FROM aggregated a
INNER JOIN filtered f ON a.email_domain = f.email_domain

