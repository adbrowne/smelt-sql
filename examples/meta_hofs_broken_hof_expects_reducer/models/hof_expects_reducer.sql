-- Intentional error: reduce's second argument must be a registered reducer name,
-- not a lambda. `reduce([1, 2, 3], fn c => c)` passes a lambda where a reducer
-- name (like plus_chain, and_all, etc.) is required.
-- Emits: HofExpectsReducer
SELECT reduce([1, 2, 3], fn c => c)
