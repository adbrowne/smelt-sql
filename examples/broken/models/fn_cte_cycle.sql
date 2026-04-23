-- Phase 20 fixture: demonstrates CTE cycle detection (CteCycle diagnostic).
-- CTEs a and b mutually reference each other, forming a cycle.
smelt.define bad_cte(
    source: TableExpr
) -> TableExpr AS (
    WITH a AS (SELECT * FROM b),
         b AS (SELECT * FROM a)
    SELECT * FROM a
)
