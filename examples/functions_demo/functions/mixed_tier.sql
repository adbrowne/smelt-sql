-- Phase 26: Tier 2 function calling a Tier 1 helper.
-- `compute_discount` is Tier 1 (unannotated). `apply_discount` is Tier 2.
smelt.define compute_discount(base_price, rate) AS (base_price * (1 - rate))

smelt.define apply_discount(
    price: Expr<Numeric>,
    rate: Expr<Numeric>
) AS (
    smelt.fn.compute_discount(price, rate)
)
