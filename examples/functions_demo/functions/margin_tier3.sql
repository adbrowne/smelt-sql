-- Phase 24 fixture: Tier 3 (fully annotated — parameters AND return type).
-- Body is checked against both declared param types and declared return type.
smelt.define margin_tier3(
    revenue: Expr<Numeric>,
    cost: Expr<Numeric>
) -> Expr<Double> AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE CAST(revenue - cost AS DOUBLE) / CAST(cost AS DOUBLE)
    END
)
