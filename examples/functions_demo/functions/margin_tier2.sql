-- Phase 23 fixture: Tier 2 (parameters annotated, return inferred).
-- Body is checked in isolation against declared parameter types.
smelt.define margin_tier2(
    revenue: Expr<Numeric>,
    cost: Expr<Numeric>
) AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE (revenue - cost) / cost
    END
)
