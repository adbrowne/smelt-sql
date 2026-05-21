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
    SELECT ip_address, cohort_date, email_domain
    FROM smelt.sql_l1_179
    WHERE quantity > 0
),
aggregated AS (
    SELECT ip_address, COUNT(*) AS cnt
    FROM filtered
    GROUP BY ip_address
)
SELECT
    a.ip_address,
    a.cnt,
    f.cohort_date
FROM aggregated a
INNER JOIN filtered f ON a.ip_address = f.ip_address

