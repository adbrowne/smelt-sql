-- Phase 17 upgrade of the Phase-15 fixture: call
-- `smelt.fn.add_margin(smelt.ref('orders'))` and project explicit
-- columns through the inferred TableExpr return schema. The
-- `order_id` column comes from `source.*` expansion, `margin`
-- comes from the explicit projection in `add_margin`'s body.
--
-- This proves Phase 17's return-schema inference wired up
-- correctly — explicit columns from a `TableExpr`-returning call
-- resolve against the caller's FROM-scope entry seeded via
-- `RefSchemaProvider::smelt_fn_columns`.
SELECT order_id, margin
FROM smelt.fn.add_margin(smelt.ref('orders')) AS m
