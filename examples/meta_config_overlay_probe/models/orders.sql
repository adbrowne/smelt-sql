---
materialization: table
---
SELECT id, region, revenue
FROM (VALUES
    (1, 'us-west-2', 150),
    (2, 'us-west-2', 80)
) AS t(id, region, revenue)
