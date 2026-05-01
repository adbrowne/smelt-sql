-- Phase 20 fixture: demonstrates multi-splice intersection via `pred`.
-- `pred` appears in both WHERE (scope = all source columns) and HAVING
-- (scope = GROUP BY key + aggregate), so its inferred context is the
-- intersection: only the columns that appear in both scopes.
smelt.define pred_demo(
    source: TableExpr,
    pred: Expr<Boolean>
) -> TableExpr AS (
    SELECT id, SUM(amount) AS total FROM source WHERE pred GROUP BY id HAVING pred
)

