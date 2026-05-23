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
    SELECT region, email_domain, price
    FROM smelt.sql_l3_143
    WHERE country = 'US'
),
aggregated AS (
    SELECT region, COUNT(*) AS cnt
    FROM filtered
    GROUP BY region
)
SELECT
    a.region,
    a.cnt,
    f.email_domain
FROM aggregated a
INNER JOIN filtered f ON a.region = f.region
