-- Phase 5 broken fixture: the body references `z`, which is not a declared
-- parameter. Emits one `UnknownIdentifier` diagnostic on the `z` span.
smelt.define bad_ref(x: Expr<Integer>, y: Expr<Integer>) AS (x + z)
