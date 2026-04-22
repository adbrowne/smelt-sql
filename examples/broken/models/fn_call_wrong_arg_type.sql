-- Phase 6 broken fixture: a Text literal is passed to a `Expr<Numeric>`
-- parameter. The call site must emit exactly one `ArgTypeMismatch`
-- diagnostic, anchored at the `'text'` argument span.
smelt.define needs_number(x: Expr<Numeric>) -> Expr<Numeric> AS (x + 1)

SELECT smelt.fn.needs_number('text') AS r
