-- Phase 5: canonical end-to-end `safe_divide` from §2 of the smelt-functions
-- research. Body is type-clean — `Expr<Numeric>` params bind to the widest
-- numeric (Double) in Tier 1, and CAST(numerator AS DOUBLE) / denominator
-- type-checks cleanly. functions_demo must remain diagnostic-clean.
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double>
    AS (CASE WHEN denominator = 0 THEN NULL ELSE CAST(numerator AS DOUBLE) / denominator END)
