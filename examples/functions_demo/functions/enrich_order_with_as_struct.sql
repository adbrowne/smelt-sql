---
deterministic: true
---
smelt.define enrich_order_with_as_struct(
  orders: TableExpr<{order_id: BigInt, customer_id: Text, total: Numeric}>,
  customers: TableExpr<{customer_id: Text, customer_name: Text, customer_tier: Text}>
) -> TableExpr AS (
  -- Phase 38 demo: smelt.as_struct() avoids column-name collisions in
  -- multi-join results. Each source's columns are bundled into a typed
  -- struct so the caller can unpack them without ambiguity.
  SELECT
    smelt.as_struct(o EXCEPT customer_id) AS order_data,
    smelt.as_struct(c EXCEPT customer_id) AS customer_data
  FROM orders AS o
  JOIN customers AS c ON o.customer_id = c.customer_id
)
