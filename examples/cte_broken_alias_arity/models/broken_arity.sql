-- One column alias but two SELECT columns in the CTE body.
-- Emits: AliasColumnArityMismatch
WITH cte(a) AS (SELECT CAST(1 AS INTEGER), CAST(2 AS INTEGER))
SELECT a FROM cte
