---
materialization: table
---
-- Synthetic orders fixture: three regions, six rows.
-- Provides the upstream data for per-cohort-union generator emissions.
SELECT
    id,
    user_id,
    region,
    revenue,
    created_at
FROM (VALUES
    (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)),
    (2, 11, 'us-west-2', 80,  CAST('2024-01-02' AS TIMESTAMP)),
    (3, 20, 'us-east-1', 120, CAST('2024-01-03' AS TIMESTAMP)),
    (4, 21, 'us-east-1', 90,  CAST('2024-01-04' AS TIMESTAMP)),
    (5, 30, 'eu-west-1', 60,  CAST('2024-01-05' AS TIMESTAMP)),
    (6, 31, 'eu-west-1', 40,  CAST('2024-01-06' AS TIMESTAMP))
) AS t(id, user_id, region, revenue, created_at)
