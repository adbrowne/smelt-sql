-- Phase 12 canonical broken fixture for the multi-level frame renderer.
-- The chain is:
--   `outer_call(y) AS (smelt.functions.middle(y))`
--   `middle(z) AS (smelt.functions.inner_unary(z))`
--   `inner_unary(x) AS (x + undefined_var)`
--
-- The innermost body contains an `undefined_var` identifier that does not
-- resolve to any parameter or scope. That triggers an `UnknownIdentifier`
-- body-cascade inside `inner_unary`, which carries one frame for
-- `inner_unary`. The body re-walks in `middle` and `outer_call` then
-- append their own frames, yielding three expansion frames total on the
-- diagnostic. The LSP renderer emits them outer-to-inner.
smelt.define inner_unary(x) AS (x + undefined_var)
smelt.define middle(z) AS (smelt.functions.inner_unary(z))
smelt.define outer_call(y) AS (smelt.functions.middle(y))

SELECT smelt.functions.outer_call(1) AS threaded
