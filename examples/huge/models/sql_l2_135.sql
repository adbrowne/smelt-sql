---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT ip_address, cohort_date, email_domain
    FROM smelt.ref('py_l1_423')
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
