---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT region, email_domain, price
    FROM smelt.ref('py_l3_467')
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
