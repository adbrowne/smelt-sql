---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    SUM(revenue) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.ref('sql_l2_222')
GROUP BY cohort_date
