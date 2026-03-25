---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    MAX(created_at) AS agg_0,
    SUM(amount) AS agg_1
FROM smelt.ref('sql_l3_108')
GROUP BY page_path
