-- Phase 45 fixture: a function whose body LEFT JOINs `dim_y` via a
-- `smelt.ref('Y') AS y` alias, but the body references a column
-- (`does_not_exist`) that's not on the joined schema.  Phase 45 makes
-- joined-alias columns visible inside the body, so unknown columns on
-- those aliases now surface as `UnknownIdentifier` rooted at the call
-- site (rather than passing silently).
--
-- The companion `fn_join_alias_missing_col_other.sql` provides the
-- `dim_y` model the function joins against — it has `id` for the join
-- predicate but no `does_not_exist`.

smelt.define join_alias_missing_local(orders: TableExpr<{order_id: BigInt}>) -> TableExpr AS (
  SELECT orders.order_id, y.does_not_exist
  FROM orders
  LEFT JOIN smelt.ref('fn_join_alias_missing_col_other') AS y
    ON orders.order_id = y.id
)

SELECT *
FROM smelt.fn.join_alias_missing_local(
  smelt.ref('fn_join_alias_missing_col_other')
) AS m
