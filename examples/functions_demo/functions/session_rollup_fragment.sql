-- Phase 21 fixture: demonstrates valid fragment context binding.
-- `filters` is spliced in WHERE (scope = all source columns), so any
-- column from `source` is valid in a caller-provided `filters` fragment.
smelt.define session_rollup_fragment(
    source: TableExpr,
    filters: Expr<Boolean>
) -> TableExpr AS (
    SELECT * FROM source WHERE filters
)

