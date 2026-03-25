---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    region,
    browser,
    revenue,
    event_date
FROM smelt.ref('sql_l3_121')
WHERE country = 'US'
