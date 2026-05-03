-- Phase 45 fixture: a function declares an `Expr<Text>` parameter
-- named `name` that collides with a column on a joined alias's
-- schema.  Phase 45 extends the shadow-warning check to consult
-- joined-alias schemas, so the warning anchors at the parameter
-- declaration even though the colliding column comes from the
-- joined alias rather than a sibling TableExpr parameter.
--
-- Companion `fn_join_alias_shadow_other.sql` supplies the joined
-- model (and doubles as the orders source).

smelt.define shadow_join_local(name: Expr<Text>, orders: TableExpr<{order_id: BigInt}>) -> TableExpr AS (
  SELECT orders.order_id, name AS computed_name
  FROM orders
  LEFT JOIN smelt.fn_join_alias_shadow_other AS y
    ON orders.order_id = y.id
)

SELECT *
FROM smelt.shadow_join_local(
  'hi',
  smelt.fn_join_alias_shadow_other
) AS m
