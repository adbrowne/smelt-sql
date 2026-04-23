-- Phase 15 fixture: call `smelt.fn.add_margin(smelt.ref('orders'))` in a
-- FROM clause. `add_margin`'s `source: TableExpr` parameter binds the
-- caller-supplied `{order_id, revenue, cost}` schema; bare `revenue` and
-- `cost` inside the body resolve through the caller schema.
--
-- We project `*` rather than named columns here because full output-
-- schema inference for `TableExpr`-returning calls lands in Phase 17
-- (research §3 / plan §17). Until then the outer SELECT cannot resolve
-- `m.margin` as a known column — the Phase-15 scope is purely the body
-- check, which this fixture exercises via the call-site expansion.
SELECT *
FROM smelt.fn.add_margin(smelt.ref('orders')) AS m
