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
    SELECT email_domain, channel, revenue
    FROM smelt.page_views
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT email_domain, COUNT(*) AS cnt
    FROM filtered
    GROUP BY email_domain
)
SELECT
    a.email_domain,
    a.cnt,
    f.channel
FROM aggregated a
INNER JOIN filtered f ON a.email_domain = f.email_domain

