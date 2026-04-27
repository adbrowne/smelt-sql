-- Phase 24 broken fixture: Tier 3 return type mismatch.
-- Body returns Text but declaration says Expr<Integer>.
smelt.define tier3_mismatch(
    x: Expr<Integer>
) -> Expr<Integer> AS (
    'not_an_integer'
)
