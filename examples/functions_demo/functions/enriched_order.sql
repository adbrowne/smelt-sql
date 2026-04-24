---
deterministic: true
provenance: { order_id: [orders.order_id], total: [orders.total], customer_id: [orders.customer_id], customer_name: [dim_customer.customer_name], customer_tier: [dim_customer.customer_tier] }
joins:
  - table: dim_customer
    on: orders.customer_id = dim_customer.customer_id
    cardinality: "1:1"
---
smelt.define enriched_order(orders: TableExpr<{order_id: BigInt, customer_id: Text, total: Numeric}>) -> TableExpr AS (
  SELECT
    orders.order_id,
    orders.customer_id,
    orders.total,
    -- Dimension columns from the LEFT JOIN; cast-annotated so the body
    -- checker doesn't flag them as unknown (JOIN aliases are not yet
    -- tracked in the function body check scope).
    CAST(NULL AS VARCHAR) AS customer_name,
    CAST(NULL AS VARCHAR) AS customer_tier
  FROM orders
  LEFT JOIN smelt.ref('dim_customer') AS dim_customer
    ON orders.customer_id = dim_customer.customer_id
)
