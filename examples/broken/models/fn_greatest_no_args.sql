-- Phase 8 broken fixture (landed in Phase 10 after the unified resolver):
-- `GREATEST` is a variadic built-in requiring at least one argument. An
-- empty call-list violates the `MissingArgs` arity check and yields a
-- `MissingArgument` diagnostic from the `unify_call` built-in branch.
SELECT smelt.GREATEST() AS r
