-- Phase 19 fixture: demonstrates context-binding syntax `Expr<T, ctx>`.
-- `filters: Expr<Boolean, source>` declares that `filters` is a boolean
-- expression whose column references are drawn from the `source` TableExpr
-- parameter. Full `session_rollup` implementation lands in Phase 22.
smelt.define session_rollup_stub(
    source: TableExpr,
    filters: Expr<Boolean, source>
) -> TableExpr AS (
    SELECT * FROM source WHERE filters
)

