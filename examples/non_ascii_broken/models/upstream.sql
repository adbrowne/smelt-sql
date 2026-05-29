---
materialization: table
---
-- Upstream fixture: provides a typed schema for the broken generator body.
-- Columns: id (INTEGER), amount (DOUBLE).
SELECT
    CAST(1 AS INTEGER) AS id,
    CAST(99.5 AS DOUBLE) AS amount
FROM (VALUES (1, 99.5)) AS t(id, amount)
