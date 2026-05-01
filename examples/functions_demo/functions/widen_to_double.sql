-- Phase 27 fixture: bidirectional widening via expected return type.
-- ABS<T: Numeric>(T) → T is in REGISTRY_MIGRATED; with the declared
-- `-> Expr<Double>` return, `expected_return = Some(Double)` propagates
-- into `try_registry_inference` so LUB(Integer, Double) = Double, and
-- the body check passes without a ReturnTypeMismatch.
smelt.define widen_to_double(x: Expr<Integer>) -> Expr<Double> AS (ABS(x))

