---
materialization: view
---
SELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label
UNION ALL SELECT CAST(2 AS BIGINT), 'beta'
UNION ALL SELECT CAST(3 AS BIGINT), 'gamma'
