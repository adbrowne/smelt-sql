-- Phase 5 broken fixture: `smelt.define` body adds an Integer param to a Text
-- literal. Should emit exactly one `FunctionBodyTypeMismatch` diagnostic at
-- the inner `x + 'text'` subexpression.
smelt.define bad_add(x: Expr<Integer>) AS (x + 'text')
