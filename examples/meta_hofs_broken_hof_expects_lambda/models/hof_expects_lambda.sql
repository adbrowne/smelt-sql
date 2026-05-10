-- Intentional error: map's second argument must be a lambda, not a literal.
-- `map([1, 2, 3], 42)` passes an integer where a lambda is required.
-- Emits: HofExpectsLambda
SELECT map([1, 2, 3], 42)
