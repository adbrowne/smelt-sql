-- Phase 8 broken fixture (landed in Phase 10 after the unified resolver):
-- `COALESCE` is generic with all arguments sharing a single type variable
-- `T`. Passing a Text arg and an Integer arg violates the variable's
-- consistency constraint and yields an `ArgTypeMismatch` diagnostic from
-- the `unify_call` built-in branch.
SELECT smelt.fn.COALESCE('text', 1) AS r
