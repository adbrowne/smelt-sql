---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    COUNT(DISTINCT user_id) AS agg_0,
    MAX(created_at) AS agg_1,
    MIN(created_at) AS agg_2
FROM smelt.ref('sql_l1_156')
GROUP BY page_path
